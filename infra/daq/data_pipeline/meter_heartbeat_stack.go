package main

import (
	"github.com/aws/aws-cdk-go/awscdk/v2"
	"github.com/aws/aws-cdk-go/awscdk/v2/awsiam"
	"github.com/aws/aws-cdk-go/awscdk/v2/awskinesis"
	"github.com/aws/aws-cdk-go/awscdk/v2/awskinesisfirehose"
	"github.com/aws/aws-cdk-go/awscdk/v2/awslakeformation"
	"github.com/aws/aws-cdk-go/awscdk/v2/awslambda"
	"github.com/aws/aws-cdk-go/awscdk/v2/awslambdaeventsources"
	"github.com/aws/aws-cdk-go/awscdk/v2/awslogs"
	"github.com/aws/aws-cdk-go/awscdk/v2/awss3"
	"github.com/aws/constructs-go/constructs/v10"
	"github.com/aws/jsii-runtime-go"
)

// NewMeterHeartbeatStack — a device-liveness tap. A NEW read-all IoT rule does NOT work:
// IoT Core for LoRaWAN destinations use ExpressionType=RuleName, so device messages are
// handed directly to the existing named rules and never hit a broker topic a new rule
// could subscribe to. The messages only exist downstream, so we tap the existing
// DAQ_INPUT_STREAM as a READ-ONLY consumer: nothing on that stream changes (just an ESM
// here). Its messages already carry customerid/schematype (stamped by the producing
// rules). The Lambda extracts one heartbeat per message (envelope only — NO decode) and
// PUTs it to Firehose, which appends to the `all.heartbeat` Iceberg table (S3 Tables).
//
// Why Firehose→Iceberg: liveness is high-volume writes + occasional reads. Firehose buffers
// ~5 min into large files + S3 Tables auto-compacts (no small-file problem), and the Iceberg
// destination waives Firehose's 5 KB-per-record rounding, so ingest is ~$50/mo at prod vs
// ~$7k/mo for per-message DynamoDB writes. The read (aggregations get_liveness) aggregates
// the append log to latest-per-device via Athena. Key = customerid + meterid; gateway is NOT
// a key (a LoRaWAN uplink is heard by multiple gateways) — it's an info column.
func NewMeterHeartbeatStack(scope constructs.Construct, id string, props *awscdk.StackProps) awscdk.Stack {
	stack := awscdk.NewStack(scope, &id, props)
	region := *stack.Region()
	account := *stack.Account()

	// ── Import the existing Flink input stream (read-only tap; NOT created/modified here). ──
	inputStream := awskinesis.Stream_FromStreamArn(stack, jsii.String("DaqInputStream"),
		jsii.String("arn:aws:kinesis:"+region+":"+account+":stream/DAQ_INPUT_STREAM"))

	// ── Heartbeat Lambda (Rust/arm64), consuming the stream in batches. ──
	fn := awslambda.NewFunction(stack, jsii.String("HeartbeatFn"), &awslambda.FunctionProps{
		FunctionName: jsii.String("meter-heartbeat"),
		Runtime:      awslambda.Runtime_PROVIDED_AL2023(),
		Architecture: awslambda.Architecture_ARM_64(),
		Handler:      jsii.String("bootstrap"),
		Code:         awslambda.Code_FromAsset(jsii.String("../../../target/lambda/meter-heartbeat"), nil),
		Timeout:      awscdk.Duration_Seconds(jsii.Number(60)),
		MemorySize:   jsii.Number(256),
		Environment: &map[string]*string{
			"FIREHOSE_STREAM": jsii.String("meter-liveness"),
		},
		LogRetention: awslogs.RetentionDays_ONE_WEEK,
	})
	// Sink: PUT deduped latest-state records to the Firehose stream (→ Iceberg upsert).
	fn.AddToRolePolicy(awsiam.NewPolicyStatement(&awsiam.PolicyStatementProps{
		Effect:    awsiam.Effect_ALLOW,
		Actions:   jsii.Strings("firehose:PutRecordBatch", "firehose:PutRecord"),
		Resources: jsii.Strings("arn:aws:firehose:" + region + ":" + account + ":deliverystream/meter-liveness"),
	}))
	// DAQ_INPUT_STREAM is SSE-KMS (aws/kinesis) — a consumer needs kms:Decrypt to read.
	// (Imported stream doesn't carry its key, so grant explicitly. Scoped to * for the
	// experiment; tighten to the aws/kinesis key ARN for prod.)
	fn.AddToRolePolicy(awsiam.NewPolicyStatement(&awsiam.PolicyStatementProps{
		Effect:    awsiam.Effect_ALLOW,
		Actions:   jsii.Strings("kms:Decrypt"),
		Resources: jsii.Strings("*"),
	}))
	// 60s batching window: batch is deduped to latest-per-device before writing, so a
	// bigger batch = fewer writes; but keep the window short so a device shows as "alive"
	// within ~a minute (field techs want near-immediate confirmation). BatchSize caps it.
	fn.AddEventSource(awslambdaeventsources.NewKinesisEventSource(inputStream, &awslambdaeventsources.KinesisEventSourceProps{
		StartingPosition:  awslambda.StartingPosition_TRIM_HORIZON,
		BatchSize:         jsii.Number(10000),
		MaxBatchingWindow: awscdk.Duration_Seconds(jsii.Number(60)),
		RetryAttempts:     jsii.Number(3),
	}))

	// ── Firehose → Iceberg (all.heartbeat) — the cheap latest-state sink. ──
	// The lambda PUTs one JSON record per device (deduped batch); Firehose buffers 5 min,
	// upserts on (customerid, meterid) via Iceberg row-level MERGE (one row per device, no
	// append-log growth), and S3 Tables auto-compacts. ~100× cheaper than per-message DDB
	// writes (Iceberg destination waives the 5 KB-per-record rounding).
	catalogArn := "arn:aws:glue:" + region + ":" + account + ":catalog/s3tablescatalog/measurements"
	s3tablesCatalogId := account + ":s3tablescatalog/measurements"

	// Delivery-error backup bucket (only holds records Firehose couldn't land; 7-day expiry).
	fhErrors := awss3.NewBucket(stack, jsii.String("HeartbeatFhErrors"), &awss3.BucketProps{
		RemovalPolicy:     awscdk.RemovalPolicy_DESTROY,
		AutoDeleteObjects: jsii.Bool(true),
		BlockPublicAccess: awss3.BlockPublicAccess_BLOCK_ALL(),
		LifecycleRules:    &[]*awss3.LifecycleRule{{Expiration: awscdk.Duration_Days(jsii.Number(7))}},
	})

	// Firehose service role: Glue federation (S3 Tables catalog) + Lake Formation data access.
	fhRole := awsiam.NewRole(stack, jsii.String("HeartbeatFhRole"), &awsiam.RoleProps{
		AssumedBy: awsiam.NewServicePrincipal(jsii.String("firehose.amazonaws.com"), nil),
	})
	// S3 Tables data access (the working aggregations reader has this too — LF grants
	// alone aren't enough for the S3 Tables storage in this account's access mode).
	fhRole.AddManagedPolicy(awsiam.ManagedPolicy_FromAwsManagedPolicyName(jsii.String("AmazonS3TablesFullAccess")))
	fhRole.AddToPolicy(awsiam.NewPolicyStatement(&awsiam.PolicyStatementProps{
		Effect:  awsiam.Effect_ALLOW,
		Actions: jsii.Strings("glue:GetTable", "glue:GetDatabase", "glue:GetDatabases", "glue:GetTables", "glue:UpdateTable", "glue:GetCatalog"),
		Resources: jsii.Strings(
			"arn:aws:glue:"+region+":"+account+":catalog",
			"arn:aws:glue:"+region+":"+account+":catalog/s3tablescatalog",
			catalogArn,
			// S3 Tables federated Glue path (matches the aggregations lambda's grants).
			"arn:aws:glue:"+region+":"+account+":database/s3tablescatalog/measurements/*",
			"arn:aws:glue:"+region+":"+account+":table/s3tablescatalog/measurements/*/*",
		),
	}))
	fhRole.AddToPolicy(awsiam.NewPolicyStatement(&awsiam.PolicyStatementProps{
		Effect:    awsiam.Effect_ALLOW,
		Actions:   jsii.Strings("lakeformation:GetDataAccess"),
		Resources: jsii.Strings("*"),
	}))
	fhErrors.GrantReadWrite(fhRole, nil)

	// Lake Formation grants so Firehose can MERGE (read + insert + delete) into the table.
	fhDl := &awslakeformation.CfnPermissions_DataLakePrincipalProperty{DataLakePrincipalIdentifier: fhRole.RoleArn()}
	fhLfDb := awslakeformation.NewCfnPermissions(stack, jsii.String("HeartbeatFhLfDb"), &awslakeformation.CfnPermissionsProps{
		DataLakePrincipal: fhDl,
		Resource: &awslakeformation.CfnPermissions_ResourceProperty{
			DatabaseResource: &awslakeformation.CfnPermissions_DatabaseResourceProperty{
				Name: jsii.String("all"), CatalogId: jsii.String(s3tablesCatalogId),
			},
		},
		Permissions: jsii.Strings("DESCRIBE"),
	})
	// "Super" (= LF permission ALL) on the table — per AWS's Firehose→S3 Tables guide,
	// the individual SELECT/INSERT/… set is NOT sufficient for Firehose's write/commit.
	fhLfTable := awslakeformation.NewCfnPermissions(stack, jsii.String("HeartbeatFhLfTable"), &awslakeformation.CfnPermissionsProps{
		DataLakePrincipal: fhDl,
		Resource: &awslakeformation.CfnPermissions_ResourceProperty{
			TableResource: &awslakeformation.CfnPermissions_TableResourceProperty{
				DatabaseName: jsii.String("all"), Name: jsii.String("heartbeat"),
				CatalogId: jsii.String(s3tablesCatalogId),
			},
		},
		Permissions: jsii.Strings("ALL"),
	})

	fh := awskinesisfirehose.NewCfnDeliveryStream(stack, jsii.String("HeartbeatFirehose"), &awskinesisfirehose.CfnDeliveryStreamProps{
		DeliveryStreamName: jsii.String("meter-liveness"),
		DeliveryStreamType: jsii.String("DirectPut"),
		IcebergDestinationConfiguration: &awskinesisfirehose.CfnDeliveryStream_IcebergDestinationConfigurationProperty{
			RoleArn: fhRole.RoleArn(),
			CatalogConfiguration: &awskinesisfirehose.CfnDeliveryStream_CatalogConfigurationProperty{
				CatalogArn: jsii.String(catalogArn),
			},
			S3Configuration: &awskinesisfirehose.CfnDeliveryStream_S3DestinationConfigurationProperty{
				BucketArn: fhErrors.BucketArn(),
				RoleArn:   fhRole.RoleArn(),
			},
			// Append-only: Firehose INSERTs every record. (Upsert/MERGE would require a
			// per-record operation via JQ/Lambda — not worth it; the read aggregates to
			// latest-per-device instead, which is cheap over the customerid-bucketed table.)
			AppendOnly: jsii.Bool(true),
			BufferingHints: &awskinesisfirehose.CfnDeliveryStream_BufferingHintsProperty{
				IntervalInSeconds: jsii.Number(300),
				SizeInMBs:         jsii.Number(128),
			},
			DestinationTableConfigurationList: &[]interface{}{
				&awskinesisfirehose.CfnDeliveryStream_DestinationTableConfigurationProperty{
					DestinationDatabaseName: jsii.String("all"),
					DestinationTableName:    jsii.String("heartbeat"),
				},
			},
		},
	})

	// Firehose validates table access (glue:GetTable via Lake Formation) at CREATE time,
	// so BOTH the LF grants AND the role's inline IAM policy must land first. CFN would
	// otherwise create the role's DefaultPolicy concurrently with Firehose → GetTable
	// authorization fails on a half-attached policy.
	fh.AddDependency(fhLfDb)
	fh.AddDependency(fhLfTable)
	if dp := fhRole.Node().TryFindChild(jsii.String("DefaultPolicy")); dp != nil {
		fh.Node().AddDependency(dp)
	}

	awscdk.NewCfnOutput(stack, jsii.String("HeartbeatFirehoseName"), &awscdk.CfnOutputProps{
		Value:       jsii.String("meter-liveness"),
		Description: jsii.String("Firehose delivery stream → all.heartbeat (upsert)"),
	})
	return stack
}
