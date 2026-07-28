package main

import (
	"os"

	"github.com/aws/aws-cdk-go/awscdk/v2"
	"github.com/aws/aws-cdk-go/awscdk/v2/awsapigatewayv2"
	"github.com/aws/aws-cdk-go/awscdk/v2/awsapigatewayv2integrations"
	"github.com/aws/aws-cdk-go/awscdk/v2/awscloudwatch"
	"github.com/aws/aws-cdk-go/awscdk/v2/awsdynamodb"
	"github.com/aws/aws-cdk-go/awscdk/v2/awsiam"
	"github.com/aws/aws-cdk-go/awscdk/v2/awslambda"
	"github.com/aws/aws-cdk-go/awscdk/v2/awslambdaeventsources"
	"github.com/aws/aws-cdk-go/awscdk/v2/awslogs"
	"github.com/aws/aws-cdk-go/awscdk/v2/awssqs"
	"github.com/aws/aws-cdk-go/awscdk/v2/awsssm"
	"github.com/aws/constructs-go/constructs/v10"
	"github.com/aws/jsii-runtime-go"
)

// EMS account A — owns sensor-identity. The role + table ARNs are stable.
const (
	emsAccount             = "891377204778"
	emsRegion              = "eu-central-1"
	emsWriterRoleArn       = "arn:aws:iam::" + emsAccount + ":role/OcamlBridgeWriterRole"
	emsSensorIdentityTable = "sensor-identity"

	// Cognito user pool in THIS account that the frontend authenticates against.
	userPoolID = "eu-central-1_gADB2vK24"
)

type OcamlHierarchyStackProps struct {
	awscdk.StackProps
}

func main() {
	defer jsii.Close()

	app := awscdk.NewApp(nil)

	NewOcamlHierarchyStack(app, "OcamlHierarchyStack", &OcamlHierarchyStackProps{
		awscdk.StackProps{
			Env:         env(),
			Description: jsii.String("EMS OCaml Hierarchy Service - Lambda and API Gateway"),
		},
	})

	app.Synth(nil)
}

func env() *awscdk.Environment {
	return &awscdk.Environment{
		Account: jsii.String(os.Getenv("CDK_DEFAULT_ACCOUNT")),
		Region:  jsii.String(os.Getenv("CDK_DEFAULT_REGION")),
	}
}

func NewOcamlHierarchyStack(scope constructs.Construct, id string, props *OcamlHierarchyStackProps) awscdk.Stack {
	var sprops awscdk.StackProps
	if props != nil {
		sprops = props.StackProps
	}
	stack := awscdk.NewStack(scope, &id, &sprops)

	tableName := "hierarchy_new"

	// Single-table DynamoDB store. NEW_AND_OLD_IMAGES streams feed
	// downstream consumers (timeseries, audit) the full before/after row.
	// RETAIN on destroy so a stack rebuild can't accidentally drop data.
	table := awsdynamodb.NewTable(stack, jsii.String("HierarchyTable"), &awsdynamodb.TableProps{
		TableName: jsii.String(tableName),
		PartitionKey: &awsdynamodb.Attribute{
			Name: jsii.String("pk"),
			Type: awsdynamodb.AttributeType_STRING,
		},
		SortKey: &awsdynamodb.Attribute{
			Name: jsii.String("sk"),
			Type: awsdynamodb.AttributeType_STRING,
		},
		BillingMode:   awsdynamodb.BillingMode_PAY_PER_REQUEST,
		Stream:        awsdynamodb.StreamViewType_NEW_AND_OLD_IMAGES,
		RemovalPolicy: awscdk.RemovalPolicy_RETAIN,
	})

	table.AddGlobalSecondaryIndex(&awsdynamodb.GlobalSecondaryIndexProps{
		IndexName: jsii.String("gsi1"),
		PartitionKey: &awsdynamodb.Attribute{
			Name: jsii.String("gsi1pk"),
			Type: awsdynamodb.AttributeType_STRING,
		},
		SortKey: &awsdynamodb.Attribute{
			Name: jsii.String("gsi1sk"),
			Type: awsdynamodb.AttributeType_STRING,
		},
		ProjectionType: awsdynamodb.ProjectionType_ALL,
	})

	// ── Cross-account read of the materialised coefficient matrix ──
	//
	// The DAQ account's measurements-aggregate Glue job assumes this to read the
	// (node, energy_type, purpose, sensor) → coefficient rows out of the `W#HN2#<id>`
	// gsi1 partition (spec §4.2, §8). Read-only, and scoped to this table.
	//
	// The trusted principal is named explicitly rather than via the account root: it is
	// the one Glue role that needs this, and MeasurementsAggregateGlueRole carries a
	// fixed RoleName precisely so this ARN stays valid across role replacements.
	readerRole := awsiam.NewRole(stack, jsii.String("HierarchyReaderRole"), &awsiam.RoleProps{
		RoleName: jsii.String("HierarchyReaderRole"),
		AssumedBy: awsiam.NewArnPrincipal(
			jsii.String("arn:aws:iam::891377204778:role/MeasurementsAggregateGlueRole")),
		Description: jsii.String("Assumed by the DAQ roll-up Glue job to read the coefficient matrix"),
	})
	readerRole.AddToPolicy(awsiam.NewPolicyStatement(&awsiam.PolicyStatementProps{
		Effect:    awsiam.Effect_ALLOW,
		Actions:   jsii.Strings("dynamodb:Query"),
		Resources: jsii.Strings(*table.TableArn(), *table.TableArn()+"/index/gsi1"),
	}))

	// ── Rust hierarchy lambda (parallel deploy, arm64) — folds Cognito in synchronously ──
	rustPoolArn := jsii.String("arn:aws:cognito-idp:" + *stack.Region() + ":" + *stack.Account() + ":userpool/" + userPoolID)
	rustRole := awsiam.NewRole(stack, jsii.String("RustHierarchyLambdaRole"), &awsiam.RoleProps{
		AssumedBy: awsiam.NewServicePrincipal(jsii.String("lambda.amazonaws.com"), nil),
		ManagedPolicies: &[]awsiam.IManagedPolicy{
			awsiam.ManagedPolicy_FromAwsManagedPolicyName(jsii.String("service-role/AWSLambdaBasicExecutionRole")),
		},
		InlinePolicies: &map[string]awsiam.PolicyDocument{
			"DynamoDBAccess": awsiam.NewPolicyDocument(&awsiam.PolicyDocumentProps{
				Statements: &[]awsiam.PolicyStatement{
					awsiam.NewPolicyStatement(&awsiam.PolicyStatementProps{
						Effect: awsiam.Effect_ALLOW,
						Actions: &[]*string{
							jsii.String("dynamodb:GetItem"), jsii.String("dynamodb:PutItem"),
							jsii.String("dynamodb:UpdateItem"), jsii.String("dynamodb:DeleteItem"),
							jsii.String("dynamodb:Query"), jsii.String("dynamodb:Scan"),
							jsii.String("dynamodb:BatchGetItem"), jsii.String("dynamodb:BatchWriteItem"),
							jsii.String("dynamodb:TransactWriteItems"),
						},
						Resources: &[]*string{table.TableArn(), jsii.String(*table.TableArn() + "/index/*")},
					}),
				},
			}),
			"CognitoAccess": awsiam.NewPolicyDocument(&awsiam.PolicyDocumentProps{
				Statements: &[]awsiam.PolicyStatement{
					awsiam.NewPolicyStatement(&awsiam.PolicyStatementProps{
						Effect: awsiam.Effect_ALLOW,
						Actions: &[]*string{
							jsii.String("cognito-idp:AdminCreateUser"),
							jsii.String("cognito-idp:AdminAddUserToGroup"),
							jsii.String("cognito-idp:AdminRemoveUserFromGroup"),
							jsii.String("cognito-idp:AdminDeleteUser"),
							jsii.String("cognito-idp:AdminGetUser"),
							jsii.String("cognito-idp:AdminSetUserPassword"),
						},
						Resources: &[]*string{rustPoolArn},
					}),
				},
			}),
		},
	})
	rustFn := awslambda.NewFunction(stack, jsii.String("RustHierarchyFunction"), &awslambda.FunctionProps{
		FunctionName: jsii.String("rust-lambda-hierarchy"),
		Runtime:      awslambda.Runtime_PROVIDED_AL2023(),
		Handler:      jsii.String("bootstrap"),
		Code:         awslambda.Code_FromAsset(jsii.String("../../target/lambda/hierarchy"), nil),
		Role:         rustRole,
		Architecture: awslambda.Architecture_ARM_64(),
		Timeout:      awscdk.Duration_Seconds(jsii.Number(30)),
		MemorySize:   jsii.Number(256),
		Environment: &map[string]*string{
			"TABLE_NAME":   jsii.String(tableName),
			"USER_POOL_ID": jsii.String(userPoolID),
		},
		Description: jsii.String("EMS Rust Hierarchy Service Lambda (arm64, parallel)"),
	})
	rustApi := awsapigatewayv2.NewHttpApi(stack, jsii.String("RustHierarchyHttpApi"), &awsapigatewayv2.HttpApiProps{
		ApiName:     jsii.String("rust-hierarchy-api"),
		Description: jsii.String("EMS Rust Hierarchy Service API"),
		CorsPreflight: &awsapigatewayv2.CorsPreflightOptions{
			AllowOrigins: &[]*string{jsii.String("*")},
			AllowMethods: &[]awsapigatewayv2.CorsHttpMethod{
				awsapigatewayv2.CorsHttpMethod_GET, awsapigatewayv2.CorsHttpMethod_POST,
				awsapigatewayv2.CorsHttpMethod_PUT, awsapigatewayv2.CorsHttpMethod_PATCH,
				awsapigatewayv2.CorsHttpMethod_DELETE, awsapigatewayv2.CorsHttpMethod_OPTIONS,
			},
			AllowHeaders: &[]*string{
				jsii.String("Content-Type"), jsii.String("X-Amz-Date"), jsii.String("Authorization"),
				jsii.String("X-Api-Key"), jsii.String("X-Amz-Security-Token"),
				jsii.String("hx-current-url"), jsii.String("hx-request"),
				jsii.String("hx-target"), jsii.String("hx-trigger"),
			},
			MaxAge: awscdk.Duration_Days(jsii.Number(1)),
		},
	})
	rustApi.AddRoutes(&awsapigatewayv2.AddRoutesOptions{
		Path:        jsii.String("/{proxy+}"),
		Methods:     &[]awsapigatewayv2.HttpMethod{awsapigatewayv2.HttpMethod_ANY},
		Integration: awsapigatewayv2integrations.NewHttpLambdaIntegration(jsii.String("RustHierarchyIntegration"), rustFn, &awsapigatewayv2integrations.HttpLambdaIntegrationProps{}),
	})
	awscdk.NewCfnOutput(stack, jsii.String("RustApiUrl"), &awscdk.CfnOutputProps{
		Value: rustApi.Url(),
	})
	// SSM param the frontend CloudFront origin reads to route /command, /query/*, /hierarchy/*
	// to the Rust lambda. Flip the frontend back to the OCaml param to roll back.
	awsssm.NewStringParameter(stack, jsii.String("RustHierarchyApiUrlParameter"), &awsssm.StringParameterProps{
		ParameterName: jsii.String("/api/rust-hierarchy-api-url"),
		StringValue:   rustApi.Url(),
		Description:   jsii.String("Rust Hierarchy API Gateway URL (frontend origin)"),
	})

	// ── Cross-account bridge: hierarchy_new DDB stream → EMS sensor-identity ──
	// Triggered by sensor-row changes in this account; assumes a role in EMS account A
	// to upsert/delete the matching sensor-identity row that Flink/Glue consume.
	bridgeRole := awsiam.NewRole(stack, jsii.String("OcamlBridgeFunctionRole"), &awsiam.RoleProps{
		AssumedBy: awsiam.NewServicePrincipal(jsii.String("lambda.amazonaws.com"), nil),
		ManagedPolicies: &[]awsiam.IManagedPolicy{
			awsiam.ManagedPolicy_FromAwsManagedPolicyName(jsii.String("service-role/AWSLambdaBasicExecutionRole")),
		},
		InlinePolicies: &map[string]awsiam.PolicyDocument{
			"AssumeEmsWriterRole": awsiam.NewPolicyDocument(&awsiam.PolicyDocumentProps{
				Statements: &[]awsiam.PolicyStatement{
					awsiam.NewPolicyStatement(&awsiam.PolicyStatementProps{
						Effect:    awsiam.Effect_ALLOW,
						Actions:   &[]*string{jsii.String("sts:AssumeRole")},
						Resources: &[]*string{jsii.String(emsWriterRoleArn)},
					}),
				},
			}),
		},
	})

	bridgeFunction := awslambda.NewFunction(stack, jsii.String("OcamlBridgeFunction"), &awslambda.FunctionProps{
		FunctionName: jsii.String("sensor-identity-bridge"),
		Runtime:      awslambda.Runtime_PYTHON_3_12(),
		Handler:      jsii.String("index.handler"),
		Code:         awslambda.Code_FromInline(jsii.String(bridgeHandlerSource)),
		Role:         bridgeRole,
		Timeout:      awscdk.Duration_Seconds(jsii.Number(30)),
		MemorySize:   jsii.Number(256),
		LogRetention: awslogs.RetentionDays_ONE_MONTH,
		Environment: &map[string]*string{
			"TARGET_ROLE_ARN": jsii.String(emsWriterRoleArn),
			"TARGET_TABLE":    jsii.String(emsSensorIdentityTable),
			"TARGET_REGION":   jsii.String(emsRegion),
		},
		Description: jsii.String("Bridges sensor changes from hierarchy_new to EMS sensor-identity"),
	})

	// Batches that survive RetryAttempts (a malformed sensor row, or a transient
	// cross-account AssumeRole / DynamoDB failure) go here instead of being
	// silently dropped off the end of the stream. For DynamoDB streams the DLQ
	// message is failure metadata (shard/sequence + error), not the row payload —
	// enough to locate and replay the offending change. 14-day retention.
	bridgeDlq := awssqs.NewQueue(stack, jsii.String("OcamlBridgeDlq"), &awssqs.QueueProps{
		QueueName:       jsii.String("sensor-identity-bridge-dlq"),
		RetentionPeriod: awscdk.Duration_Days(jsii.Number(14)),
	})

	bridgeFunction.AddEventSource(awslambdaeventsources.NewDynamoEventSource(table, &awslambdaeventsources.DynamoEventSourceProps{
		StartingPosition:   awslambda.StartingPosition_LATEST,
		BatchSize:          jsii.Number(50),
		RetryAttempts:      jsii.Number(5),
		BisectBatchOnError: jsii.Bool(true),
		// Surface poison batches instead of losing them; CDK grants the function
		// role sqs:SendMessage on this queue automatically.
		OnFailure: awslambdaeventsources.NewSqsDlq(bridgeDlq),
		// Filter at the event source: only `type=sensor` items with `sk` starting `active#`.
		Filters: &[]*map[string]interface{}{
			{
				"pattern": `{"dynamodb":{"NewImage":{"type":{"S":["sensor"]},"sk":{"S":[{"prefix":"active#"}]}}}}`,
			},
			{
				"pattern": `{"dynamodb":{"OldImage":{"type":{"S":["sensor"]},"sk":{"S":[{"prefix":"active#"}]}}}}`,
			},
		},
	}))

	// Alarm the moment anything lands in the DLQ — a non-empty DLQ means a sensor
	// change failed to propagate to sensor-identity and needs a look. Maximum over
	// a 5-min period so a single message trips it; missing data = empty queue = OK.
	bridgeDlq.MetricApproximateNumberOfMessagesVisible(&awscloudwatch.MetricOptions{
		Period:    awscdk.Duration_Minutes(jsii.Number(5)),
		Statistic: jsii.String("Maximum"),
	}).CreateAlarm(stack, jsii.String("OcamlBridgeDlqDepthAlarm"), &awscloudwatch.CreateAlarmOptions{
		AlarmName:          jsii.String("sensor-identity-bridge-dlq-not-empty"),
		AlarmDescription:   jsii.String("sensor-identity bridge DLQ has messages — sensor changes failed to propagate"),
		Threshold:          jsii.Number(1),
		EvaluationPeriods:  jsii.Number(1),
		ComparisonOperator: awscloudwatch.ComparisonOperator_GREATER_THAN_OR_EQUAL_TO_THRESHOLD,
		TreatMissingData:   awscloudwatch.TreatMissingData_NOT_BREACHING,
	})

	awscdk.NewCfnOutput(stack, jsii.String("BridgeFunctionArn"), &awscdk.CfnOutputProps{
		Value: bridgeFunction.FunctionArn(),
	})
	awscdk.NewCfnOutput(stack, jsii.String("BridgeDlqUrl"), &awscdk.CfnOutputProps{
		Value: bridgeDlq.QueueUrl(),
	})

	return stack
}

// Bridge handler — translates an active-sensor row in hierarchy_new to the
// matching sensor-identity row in EMS account A. Inlined here so the bridge has
// no external code asset; partition-key formula must match
// flink_app_scala/.../DdbBootstrapLoader.partitionKey (Java String.hashCode % 20000).
const bridgeHandlerSource = `
import json, os, boto3
from boto3.dynamodb.types import TypeDeserializer

ROLE = os.environ["TARGET_ROLE_ARN"]
TABLE = os.environ["TARGET_TABLE"]
REGION = os.environ["TARGET_REGION"]
PARTITIONS = 20000
_d = TypeDeserializer()
_creds = None

def _hash(s):
    h = 0
    for c in s:
        h = (31 * h + ord(c)) & 0xFFFFFFFF
    return h - 0x100000000 if h >= 0x80000000 else h

def _pk(daq_id):
    return f"{abs(_hash(daq_id)) % PARTITIONS:05d}"

def _is_active(img):
    return (img.get("type", {}).get("S") == "sensor"
            and img.get("sk", {}).get("S", "").startswith("active#"))

def _path(p):
    return "|".join(seg for seg in p.split("|") if not seg.startswith("S#"))

def _item(img):
    sid = int(img["pk"]["S"][2:])
    daq = img["daq_id"]["S"]
    out = {
        "pk": {"S": _pk(daq)},
        "sk": {"S": daq},
        "logical_id": {"N": str(sid)},
        "reading_kind": {"S": img["reading_kind"]["S"]},
        "hierarchy_path": {"S": _path(img["gsi1sk"]["S"])},
        "energy_type": {"S": img["energy_type"]["S"]},
    }
    # resample_minutes is optional in the source sensor row. Omit it when unset so
    # the sensor-identity row carries no resample_minutes attribute — Flink's
    # DdbBootstrapLoader / DdbStreamDeserializer treat an absent value as null
    # (raw passthrough, no resampling). A sentinel like 0 would be an invalid
    # resample interval.
    if "resample_minutes" in img:
        out["resample_minutes"] = {"N": img["resample_minutes"]["N"]}
    return out

def _client():
    global _creds
    if _creds is None:
        _creds = boto3.client("sts").assume_role(
            RoleArn=ROLE, RoleSessionName="ocaml-bridge")["Credentials"]
    return boto3.client("dynamodb", region_name=REGION,
        aws_access_key_id=_creds["AccessKeyId"],
        aws_secret_access_key=_creds["SecretAccessKey"],
        aws_session_token=_creds["SessionToken"])

def handler(event, _ctx):
    c = _client()
    for r in event["Records"]:
        ev = r["eventName"]
        ddb = r["dynamodb"]
        if ev == "REMOVE":
            old = ddb.get("OldImage", {})
            if not _is_active(old):
                continue
            daq = old["daq_id"]["S"]
            c.delete_item(TableName=TABLE,
                Key={"pk": {"S": _pk(daq)}, "sk": {"S": daq}})
        else:
            new = ddb.get("NewImage", {})
            if not _is_active(new):
                continue
            c.put_item(TableName=TABLE, Item=_item(new))
    return {"processed": len(event["Records"])}
`
