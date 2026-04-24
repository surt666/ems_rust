package main

import (
	"os"

	"github.com/aws/aws-cdk-go/awscdk/v2"
	"github.com/aws/aws-cdk-go/awscdk/v2/awsapigatewayv2"
	"github.com/aws/aws-cdk-go/awscdk/v2/awsapigatewayv2integrations"
	"github.com/aws/aws-cdk-go/awscdk/v2/awsdynamodb"
	"github.com/aws/aws-cdk-go/awscdk/v2/awsiam"
	"github.com/aws/aws-cdk-go/awscdk/v2/awslambda"
	"github.com/aws/aws-cdk-go/awscdk/v2/awsssm"
	"github.com/aws/constructs-go/constructs/v10"
	"github.com/aws/jsii-runtime-go"
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

	// Import existing DynamoDB table (populated, managed outside this stack).
	table := awsdynamodb.Table_FromTableName(stack, jsii.String("HierarchyTable"), jsii.String(tableName))

	// IAM role for Lambda — DynamoDB access only (no Cognito, no SSM).
	lambdaRole := awsiam.NewRole(stack, jsii.String("OcamlHierarchyLambdaRole"), &awsiam.RoleProps{
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
							jsii.String("dynamodb:GetItem"),
							jsii.String("dynamodb:PutItem"),
							jsii.String("dynamodb:UpdateItem"),
							jsii.String("dynamodb:DeleteItem"),
							jsii.String("dynamodb:Query"),
							jsii.String("dynamodb:Scan"),
							jsii.String("dynamodb:BatchGetItem"),
							jsii.String("dynamodb:BatchWriteItem"),
						},
						Resources: &[]*string{
							table.TableArn(),
							jsii.String(*table.TableArn() + "/index/*"),
						},
					}),
				},
			}),
		},
	})

	// OCaml lambda — prebuilt zip produced by `make build` in project root.
	lambdaFunction := awslambda.NewFunction(stack, jsii.String("OcamlHierarchyFunction"), &awslambda.FunctionProps{
		FunctionName: jsii.String("ocaml-lambda-hierarchy"),
		Runtime:      awslambda.Runtime_PROVIDED_AL2023(),
		Handler:      jsii.String("bootstrap"),
		Code:         awslambda.Code_FromAsset(jsii.String("../../ocaml-lambda-hierarchy.zip"), nil),
		Role:         lambdaRole,
		Architecture: awslambda.Architecture_ARM_64(),
		Timeout:      awscdk.Duration_Seconds(jsii.Number(30)),
		MemorySize:   jsii.Number(512),
		Environment: &map[string]*string{
			"HIERARCHY_TABLE": jsii.String(tableName),
		},
		Description: jsii.String("EMS OCaml Hierarchy Service Lambda Function"),
	})

	// HTTP API Gateway — same CORS shape as Rust version (HTMX headers included).
	httpApi := awsapigatewayv2.NewHttpApi(stack, jsii.String("OcamlHierarchyHttpApi"), &awsapigatewayv2.HttpApiProps{
		ApiName:     jsii.String("ocaml-hierarchy-api"),
		Description: jsii.String("EMS OCaml Hierarchy Service API"),
		CorsPreflight: &awsapigatewayv2.CorsPreflightOptions{
			AllowOrigins: &[]*string{jsii.String("*")},
			AllowMethods: &[]awsapigatewayv2.CorsHttpMethod{
				awsapigatewayv2.CorsHttpMethod_GET,
				awsapigatewayv2.CorsHttpMethod_POST,
				awsapigatewayv2.CorsHttpMethod_PUT,
				awsapigatewayv2.CorsHttpMethod_PATCH,
				awsapigatewayv2.CorsHttpMethod_DELETE,
				awsapigatewayv2.CorsHttpMethod_OPTIONS,
			},
			AllowHeaders: &[]*string{
				jsii.String("Content-Type"),
				jsii.String("X-Amz-Date"),
				jsii.String("Authorization"),
				jsii.String("X-Api-Key"),
				jsii.String("X-Amz-Security-Token"),
				jsii.String("hx-current-url"),
				jsii.String("hx-request"),
				jsii.String("hx-target"),
				jsii.String("hx-trigger"),
			},
			MaxAge: awscdk.Duration_Days(jsii.Number(1)),
		},
	})

	integration := awsapigatewayv2integrations.NewHttpLambdaIntegration(
		jsii.String("OcamlHierarchyIntegration"),
		lambdaFunction,
		&awsapigatewayv2integrations.HttpLambdaIntegrationProps{},
	)

	// Routes match the OCaml handler: GET /query/{action}, POST /command,
	// plus /hierarchy/* aliases (HTMX pages in the frontend hit those).
	httpApi.AddRoutes(&awsapigatewayv2.AddRoutesOptions{
		Path:        jsii.String("/query/{action}"),
		Methods:     &[]awsapigatewayv2.HttpMethod{awsapigatewayv2.HttpMethod_GET},
		Integration: integration,
	})

	httpApi.AddRoutes(&awsapigatewayv2.AddRoutesOptions{
		Path:        jsii.String("/command"),
		Methods:     &[]awsapigatewayv2.HttpMethod{awsapigatewayv2.HttpMethod_POST},
		Integration: integration,
	})

	httpApi.AddRoutes(&awsapigatewayv2.AddRoutesOptions{
		Path: jsii.String("/hierarchy/{proxy+}"),
		Methods: &[]awsapigatewayv2.HttpMethod{
			awsapigatewayv2.HttpMethod_GET,
			awsapigatewayv2.HttpMethod_POST,
			awsapigatewayv2.HttpMethod_PUT,
			awsapigatewayv2.HttpMethod_DELETE,
		},
		Integration: integration,
	})

	// SSM parameter — distinct from the Rust `/api/hierarchy-api-url`.
	awsssm.NewStringParameter(stack, jsii.String("OcamlHierarchyApiUrlParameter"), &awsssm.StringParameterProps{
		ParameterName: jsii.String("/api/ocaml-hierarchy-api-url"),
		StringValue:   httpApi.Url(),
		Description:   jsii.String("OCaml Hierarchy API Gateway URL"),
	})

	awscdk.NewCfnOutput(stack, jsii.String("ApiUrl"), &awscdk.CfnOutputProps{
		Value: httpApi.Url(),
	})
	awscdk.NewCfnOutput(stack, jsii.String("FunctionName"), &awscdk.CfnOutputProps{
		Value: lambdaFunction.FunctionName(),
	})

	return stack
}
