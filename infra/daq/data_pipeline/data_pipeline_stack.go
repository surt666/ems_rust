package main

import (
	"strconv"
	"time"

	"github.com/aws/aws-cdk-go/awscdk/v2"
	"github.com/aws/aws-cdk-go/awscdk/v2/awsdynamodb"
	"github.com/aws/aws-cdk-go/awscdk/v2/awsiam"
	"github.com/aws/aws-cdk-go/awscdk/v2/awskinesis"
	"github.com/aws/aws-cdk-go/awscdk/v2/awskinesisanalyticsv2"
	"github.com/aws/aws-cdk-go/awscdk/v2/awslogs"
	"github.com/aws/aws-cdk-go/awscdk/v2/awss3"
	"github.com/aws/aws-cdk-go/awscdk/v2/awss3deployment"
	"github.com/aws/constructs-go/constructs/v10"
	"github.com/aws/jsii-runtime-go"
)

type DaqPipelineStackProps struct {
	awscdk.StackProps
	AppName     string
	ParPerKPU   string
	MaxKPUs     string
	Sha         string
	TableBucket string
}

type DaqPipelineStack struct {
	awscdk.Stack
	ErrorStream        awskinesis.Stream
	MeterIdentityTable awsdynamodb.Table
}

func NewDaqPipelineStack(scope constructs.Construct, id string, props *DaqPipelineStackProps) *DaqPipelineStack {
	stack := awscdk.NewStack(scope, &id, &props.StackProps)
	region := *stack.Region()
	account := *stack.Account()

	tableBucketArn := "arn:aws:s3tables:" + region + ":" + account + ":bucket/" + props.TableBucket

	inputStream := awskinesis.Stream_FromStreamArn(stack, jsii.String("InputJsonStream"),
		jsii.String("arn:aws:kinesis:"+region+":"+account+":stream/DAQ_INPUT_STREAM"))

	meterIdentity := awsdynamodb.NewTable(stack, jsii.String("MeterIdentity"), &awsdynamodb.TableProps{
		TableName:           jsii.String("meter-identity"),
		PartitionKey:        &awsdynamodb.Attribute{Name: jsii.String("pk"), Type: awsdynamodb.AttributeType_STRING},
		SortKey:             &awsdynamodb.Attribute{Name: jsii.String("sk"), Type: awsdynamodb.AttributeType_STRING},
		BillingMode:         awsdynamodb.BillingMode_PAY_PER_REQUEST,
		Stream:              awsdynamodb.StreamViewType_NEW_AND_OLD_IMAGES,
		PointInTimeRecovery: jsii.Bool(true),
		RemovalPolicy:       awscdk.RemovalPolicy_RETAIN,
	})

	ddbChangeStream := awskinesis.NewStream(stack, jsii.String("DdbChangeStream"), &awskinesis.StreamProps{
		StreamName:      jsii.String(props.AppName + "-ddb-changes"),
		ShardCount:      jsii.Number(1),
		RetentionPeriod: awscdk.Duration_Hours(jsii.Number(24)),
	})

	cfnTable := meterIdentity.Node().DefaultChild().(awsdynamodb.CfnTable)
	cfnTable.AddPropertyOverride(jsii.String("KinesisStreamSpecification"), map[string]interface{}{
		"StreamArn": ddbChangeStream.StreamArn(),
	})

	errorStream := awskinesis.NewStream(stack, jsii.String("ErrorStream"), &awskinesis.StreamProps{
		StreamName:      jsii.String(props.AppName + "-errors"),
		ShardCount:      jsii.Number(1),
		RetentionPeriod: awscdk.Duration_Hours(jsii.Number(24)),
	})

	serviceRole := awsiam.NewRole(stack, jsii.String("KDAServiceRole"), &awsiam.RoleProps{
		AssumedBy:   awsiam.NewServicePrincipal(jsii.String("kinesisanalytics.amazonaws.com"), nil),
		Description: jsii.String("Role for Kinesis Data Analytics to access resources"),
		InlinePolicies: &map[string]awsiam.PolicyDocument{
			"s3Access": awsiam.NewPolicyDocument(&awsiam.PolicyDocumentProps{
				Statements: &[]awsiam.PolicyStatement{
					awsiam.NewPolicyStatement(&awsiam.PolicyStatementProps{
						Effect: awsiam.Effect_ALLOW,
						Actions: jsii.Strings("s3:GetObject*", "s3:GetBucket*", "s3:List*"),
						Resources: jsii.Strings(
							"arn:aws:s3:::flink-code-"+account+"-"+region,
							"arn:aws:s3:::flink-code-"+account+"-"+region+"/*",
						),
					}),
				},
			}),
		},
	})

	addPolicy := func(actions, resources *[]*string) {
		serviceRole.AddToPolicy(awsiam.NewPolicyStatement(&awsiam.PolicyStatementProps{
			Effect: awsiam.Effect_ALLOW, Actions: actions, Resources: resources,
		}))
	}

	// Kinesis read on input
	addPolicy(
		jsii.Strings("kinesis:GetShardIterator", "kinesis:GetRecords", "kinesis:DescribeStream",
			"kinesis:DescribeStreamSummary", "kinesis:ListShards"),
		&[]*string{inputStream.StreamArn()},
	)
	// S3 Tables for Iceberg sink
	addPolicy(
		jsii.Strings("s3tables:GetTableBucket", "s3tables:GetTable", "s3tables:GetTableMetadataLocation",
			"s3tables:UpdateTableMetadataLocation", "s3tables:PutTableData", "s3tables:GetTableData",
			"s3tables:ListTables", "s3tables:ListNamespaces", "s3tables:GetNamespace"),
		jsii.Strings(tableBucketArn, tableBucketArn+"/*"),
	)
	// S3 data files
	addPolicy(
		jsii.Strings("s3:GetObject", "s3:GetObjectVersion", "s3:PutObject", "s3:DeleteObject",
			"s3:ListBucket", "s3:ListBucketMultipartUploads", "s3:AbortMultipartUpload",
			"s3:ListMultipartUploadParts"),
		jsii.Strings("arn:aws:s3:::*/*"+account+"*", "arn:aws:s3:::*/*"+account+"*/*"),
	)
	// DDB bootstrap scan
	addPolicy(jsii.Strings("dynamodb:Query", "dynamodb:Scan"), &[]*string{meterIdentity.TableArn()})
	// DDB CDC stream
	addPolicy(
		jsii.Strings("kinesis:GetShardIterator", "kinesis:GetRecords", "kinesis:DescribeStream",
			"kinesis:DescribeStreamSummary", "kinesis:ListShards"),
		&[]*string{ddbChangeStream.StreamArn()},
	)
	// Error stream write
	addPolicy(
		jsii.Strings("kinesis:PutRecord", "kinesis:PutRecords", "kinesis:DescribeStream",
			"kinesis:DescribeStreamSummary", "kinesis:ListShards"),
		&[]*string{errorStream.StreamArn()},
	)

	logGroup := awslogs.NewLogGroup(stack, jsii.String("KDALogGroup"), &awslogs.LogGroupProps{
		LogGroupName:  jsii.String("/aws/kinesis-analytics/" + props.AppName),
		Retention:     awslogs.RetentionDays_ONE_DAY,
		RemovalPolicy: awscdk.RemovalPolicy_DESTROY,
	})
	logStream := awslogs.NewLogStream(stack, jsii.String("KdaLogStream"), &awslogs.LogStreamProps{
		LogGroup:      logGroup,
		RemovalPolicy: awscdk.RemovalPolicy_DESTROY,
	})
	logStreamArn := "arn:aws:logs:" + region + ":" + account + ":log-group:" +
		*logGroup.LogGroupName() + ":log-stream:" + *logStream.LogStreamName()

	addPolicy(
		jsii.Strings("logs:PutLogEvents", "logs:CreateLogStream", "logs:CreateLogGroup",
			"logs:DescribeLogGroups", "logs:DescribeLogStreams", "logs:GetLogEvents",
			"logs:PutRetentionPolicy"),
		jsii.Strings(
			"arn:aws:logs:"+region+":"+account+":log-group:/aws/kinesis-analytics/*",
			"arn:aws:logs:"+region+":"+account+":log-group:/aws/kinesis-analytics/*:log-stream:*",
		),
	)

	bucket := awss3.Bucket_FromBucketName(stack, jsii.String("FlinkAppsBucket"),
		jsii.String("flink-code-"+account+"-"+region))

	deployment := awss3deployment.NewBucketDeployment(stack, jsii.String("DeployFlinkApp"),
		&awss3deployment.BucketDeploymentProps{
			Sources: &[]awss3deployment.ISource{
				awss3deployment.Source_Asset(
					jsii.String("./flink_app_scala/target/scala-3.3.4/flink-app-scala-0.1.0.jar"), nil),
			},
			DestinationBucket:    bucket,
			DestinationKeyPrefix: jsii.String("flink_apps/"),
			Extract:              jsii.Bool(false),
			MemoryLimit:          jsii.Number(2048),
		})

	s3ObjectKey := "flink_apps/" + *awscdk.Fn_Select(jsii.Number(0), deployment.ObjectKeys())

	parPerKpu, _ := strconv.Atoi(props.ParPerKPU)
	if parPerKpu == 0 {
		parPerKpu = 1
	}
	maxKpu, _ := strconv.Atoi(props.MaxKPUs)
	if maxKpu == 0 {
		maxKpu = 1
	}

	flinkApp := awskinesisanalyticsv2.NewCfnApplication(stack, jsii.String("FlinkApplication"),
		&awskinesisanalyticsv2.CfnApplicationProps{
			ApplicationName:      jsii.String(props.AppName),
			RuntimeEnvironment:   jsii.String("FLINK-1_20"),
			ServiceExecutionRole: serviceRole.RoleArn(),
			RunConfiguration: &awskinesisanalyticsv2.CfnApplication_RunConfigurationProperty{
				ApplicationRestoreConfiguration: &awskinesisanalyticsv2.CfnApplication_ApplicationRestoreConfigurationProperty{
					ApplicationRestoreType: jsii.String("RESTORE_FROM_LATEST_SNAPSHOT"),
				},
				FlinkRunConfiguration: &awskinesisanalyticsv2.CfnApplication_FlinkRunConfigurationProperty{
					AllowNonRestoredState: jsii.Bool(true),
				},
			},
			ApplicationConfiguration: &awskinesisanalyticsv2.CfnApplication_ApplicationConfigurationProperty{
				FlinkApplicationConfiguration: &awskinesisanalyticsv2.CfnApplication_FlinkApplicationConfigurationProperty{
					CheckpointConfiguration: &awskinesisanalyticsv2.CfnApplication_CheckpointConfigurationProperty{
						ConfigurationType:          jsii.String("CUSTOM"),
						CheckpointingEnabled:       jsii.Bool(true),
						CheckpointInterval:         jsii.Number(300000),
						MinPauseBetweenCheckpoints: jsii.Number(10000),
					},
					MonitoringConfiguration: &awskinesisanalyticsv2.CfnApplication_MonitoringConfigurationProperty{
						ConfigurationType: jsii.String("CUSTOM"),
						LogLevel:          jsii.String("INFO"),
						MetricsLevel:      jsii.String("TASK"),
					},
					ParallelismConfiguration: &awskinesisanalyticsv2.CfnApplication_ParallelismConfigurationProperty{
						ConfigurationType:  jsii.String("CUSTOM"),
						Parallelism:        jsii.Number(float64(maxKpu)),
						ParallelismPerKpu:  jsii.Number(float64(parPerKpu)),
						AutoScalingEnabled: jsii.Bool(true),
					},
				},
				ApplicationSystemRollbackConfiguration: &awskinesisanalyticsv2.CfnApplication_ApplicationSystemRollbackConfigurationProperty{
					RollbackEnabled: jsii.Bool(true),
				},
				ApplicationSnapshotConfiguration: &awskinesisanalyticsv2.CfnApplication_ApplicationSnapshotConfigurationProperty{
					SnapshotsEnabled: jsii.Bool(true),
				},
				EnvironmentProperties: &awskinesisanalyticsv2.CfnApplication_EnvironmentPropertiesProperty{
					PropertyGroups: &[]interface{}{
						&awskinesisanalyticsv2.CfnApplication_PropertyGroupProperty{
							PropertyGroupId: jsii.String("FlinkApplicationProperties"),
							PropertyMap: &map[string]*string{
								"INPUT_STREAM":            inputStream.StreamName(),
								"AWS_REGION":              jsii.String(region),
								"ACCOUNT_ID":              jsii.String(account),
								"TABLE_BUCKET_NAME":       jsii.String(props.TableBucket),
								"METER_IDENTITY_TABLE":    meterIdentity.TableName(),
								"DDB_CHANGE_STREAM":       ddbChangeStream.StreamName(),
								"ERROR_STREAM":            errorStream.StreamName(),
								"MAX_OUT_OF_ORDERNESS_MS": jsii.String("3600000"),
								"BUFFER_RETENTION_MS":     jsii.String("21600000"),
							},
						},
						&awskinesisanalyticsv2.CfnApplication_PropertyGroupProperty{
							PropertyGroupId: jsii.String("VersionHack"),
							PropertyMap: &map[string]*string{
								"timestamp": jsii.String(time.Now().UTC().Format(time.RFC3339Nano)),
							},
						},
					},
				},
				ApplicationCodeConfiguration: &awskinesisanalyticsv2.CfnApplication_ApplicationCodeConfigurationProperty{
					CodeContent: &awskinesisanalyticsv2.CfnApplication_CodeContentProperty{
						S3ContentLocation: &awskinesisanalyticsv2.CfnApplication_S3ContentLocationProperty{
							BucketArn: bucket.BucketArn(),
							FileKey:   jsii.String(s3ObjectKey),
						},
					},
					CodeContentType: jsii.String("ZIPFILE"),
				},
			},
		})
	flinkApp.Node().AddDependency(deployment)
	awscdk.Tags_Of(flinkApp).Add(jsii.String("app"), jsii.String("FlinkProcessor"), nil)

	awskinesisanalyticsv2.NewCfnApplicationCloudWatchLoggingOption(stack, jsii.String("KdsFlinkProducerLogging"),
		&awskinesisanalyticsv2.CfnApplicationCloudWatchLoggingOptionProps{
			ApplicationName: flinkApp.Ref(),
			CloudWatchLoggingOption: &awskinesisanalyticsv2.CfnApplicationCloudWatchLoggingOption_CloudWatchLoggingOptionProperty{
				LogStreamArn: jsii.String(logStreamArn),
			},
		})

	return &DaqPipelineStack{
		Stack:              stack,
		ErrorStream:        errorStream,
		MeterIdentityTable: meterIdentity,
	}
}
