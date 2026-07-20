package main

import (
	"github.com/aws/aws-cdk-go/awscdk/v2"
	"github.com/aws/aws-cdk-go/awscdk/v2/awsdynamodb"
	"github.com/aws/aws-cdk-go/awscdk/v2/awsiam"
	"github.com/aws/aws-cdk-go/awscdk/v2/awskinesis"
	"github.com/aws/aws-cdk-go/awscdk/v2/awslambda"
	"github.com/aws/aws-cdk-go/awscdk/v2/awslambdaeventsources"
	"github.com/aws/aws-cdk-go/awscdk/v2/awslogs"
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
// upserts the LATEST state per device into the `meter-liveness` DynamoDB table.
//
// Why DynamoDB (not S3/Parquet): liveness is a latest-state lookup. Keyed
// pk=customerid / sk=meterid, it's one item per device (bounded by device count, not
// message rate) with O(1) reads — no small files, no compaction, no scan. (The earlier
// Parquet-lake design died on small-file read latency: thousands of tiny files timed the
// query out.) Gateway is NOT a key — a LoRaWAN uplink is heard by multiple gateways, so
// it isn't unique; stored as an informational column.
func NewMeterHeartbeatStack(scope constructs.Construct, id string, props *awscdk.StackProps) awscdk.Stack {
	stack := awscdk.NewStack(scope, &id, props)
	region := *stack.Region()
	account := *stack.Account()

	// ── Latest-state liveness table. pk=customerid, sk=meterid. On-demand. Demo → DESTROY.
	table := awsdynamodb.NewTable(stack, jsii.String("MeterLivenessTable"), &awsdynamodb.TableProps{
		TableName:    jsii.String("meter-liveness"),
		PartitionKey: &awsdynamodb.Attribute{Name: jsii.String("pk"), Type: awsdynamodb.AttributeType_STRING},
		SortKey:      &awsdynamodb.Attribute{Name: jsii.String("sk"), Type: awsdynamodb.AttributeType_STRING},
		BillingMode:  awsdynamodb.BillingMode_PAY_PER_REQUEST,
		RemovalPolicy: awscdk.RemovalPolicy_DESTROY,
	})

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
			"LIVENESS_TABLE": table.TableName(),
		},
		LogRetention: awslogs.RetentionDays_ONE_WEEK,
	})
	table.GrantWriteData(fn)
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

	awscdk.NewCfnOutput(stack, jsii.String("MeterLivenessTableName"), &awscdk.CfnOutputProps{
		Value:       table.TableName(),
		Description: jsii.String("Latest-state liveness table (pk=customerid, sk=meterid)"),
	})
	return stack
}
