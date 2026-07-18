package main

import (
	"github.com/aws/aws-cdk-go/awscdk/v2"
	"github.com/aws/aws-cdk-go/awscdk/v2/awsiam"
	"github.com/aws/aws-cdk-go/awscdk/v2/awskinesis"
	"github.com/aws/aws-cdk-go/awscdk/v2/awslambda"
	"github.com/aws/aws-cdk-go/awscdk/v2/awslambdaeventsources"
	"github.com/aws/aws-cdk-go/awscdk/v2/awslogs"
	"github.com/aws/aws-cdk-go/awscdk/v2/awss3"
	"github.com/aws/constructs-go/constructs/v10"
	"github.com/aws/jsii-runtime-go"
)

// NewMeterHeartbeatStack — EXPERIMENT: a device-liveness tap. A NEW read-all IoT rule was
// tried but does NOT work: IoT Core for LoRaWAN destinations use ExpressionType=RuleName,
// so device messages are handed directly to the existing named rules and never hit a
// broker topic that a new rule could subscribe to. The messages only exist downstream, so
// we tap the existing DAQ_INPUT_STREAM as a READ-ONLY consumer: nothing on that stream
// changes (no config/data/policy touched — just an ESM in this stack; removable by
// destroy). Its messages already carry customerid/schematype (stamped by the producing
// rules). The Lambda extracts one heartbeat per message (envelope only — NO decode) and
// writes JSONL to a unified S3 bucket, partitioned by ingest date, for Athena.
// Prod graduation: switch the ESM to enhanced fan-out to isolate from Flink's throughput.
func NewMeterHeartbeatStack(scope constructs.Construct, id string, props *awscdk.StackProps) awscdk.Stack {
	stack := awscdk.NewStack(scope, &id, props)
	region := *stack.Region()
	account := *stack.Account()

	// ── Unified heartbeat bucket (Parquet, dt-partitioned). Demo → DESTROY.
	// Stable, explicit name so the read side (query-raw-duck) can reference it without a
	// cross-stack import — TRIM_HORIZON re-backfills if the bucket is ever recreated.
	bucket := awss3.NewBucket(stack, jsii.String("HeartbeatBucket"), &awss3.BucketProps{
		BucketName:        jsii.String("meter-heartbeat-" + account + "-" + region),
		RemovalPolicy:     awscdk.RemovalPolicy_DESTROY,
		AutoDeleteObjects: jsii.Bool(true),
		BlockPublicAccess: awss3.BlockPublicAccess_BLOCK_ALL(),
		LifecycleRules: &[]*awss3.LifecycleRule{{
			Transitions: &[]*awss3.Transition{
				{StorageClass: awss3.StorageClass_INFREQUENT_ACCESS(), TransitionAfter: awscdk.Duration_Days(jsii.Number(30))},
				{StorageClass: awss3.StorageClass_GLACIER(), TransitionAfter: awscdk.Duration_Days(jsii.Number(120))},
			},
		}},
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
			"HEARTBEAT_BUCKET": bucket.BucketName(),
		},
		LogRetention: awslogs.RetentionDays_ONE_WEEK,
	})
	bucket.GrantWrite(fn, nil, nil)
	// DAQ_INPUT_STREAM is SSE-KMS (aws/kinesis) — a consumer needs kms:Decrypt to read.
	// (Imported stream doesn't carry its key, so grant explicitly. Scoped to * for the
	// experiment; tighten to the aws/kinesis key ARN for prod.)
	fn.AddToRolePolicy(awsiam.NewPolicyStatement(&awsiam.PolicyStatementProps{
		Effect:    awsiam.Effect_ALLOW,
		Actions:   jsii.Strings("kms:Decrypt"),
		Resources: jsii.Strings("*"),
	}))
	// TRIM_HORIZON: on first deploy, backfill the stream's retention (~24h) so every
	// device's recent activity shows up immediately (traffic is bursty ~every 15-20 min);
	// then it tails live. Small at dev volume.
	// Max the batch so each invocation writes as large a Parquet file as event-driven
	// allows (one batch = one file). 10k records / 300s window (capped by the 6 MB
	// invoke payload). Bigger files without a compaction job.
	fn.AddEventSource(awslambdaeventsources.NewKinesisEventSource(inputStream, &awslambdaeventsources.KinesisEventSourceProps{
		StartingPosition:  awslambda.StartingPosition_TRIM_HORIZON,
		BatchSize:         jsii.Number(10000),
		MaxBatchingWindow: awscdk.Duration_Seconds(jsii.Number(300)),
		RetryAttempts:     jsii.Number(3),
	}))

	awscdk.NewCfnOutput(stack, jsii.String("HeartbeatBucketName"), &awscdk.CfnOutputProps{
		Value:       bucket.BucketName(),
		Description: jsii.String("Unified heartbeat bucket (JSONL, dt-partitioned) — point Athena here"),
	})
	return stack
}
