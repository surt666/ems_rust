package main

import (
	"github.com/aws/aws-cdk-go/awscdk/v2"
	"github.com/aws/aws-cdk-go/awscdk/v2/awsdynamodb"
	"github.com/aws/aws-cdk-go/awscdk/v2/awsglue"
	"github.com/aws/aws-cdk-go/awscdk/v2/awsiam"
	"github.com/aws/aws-cdk-go/awscdk/v2/awskinesis"
	"github.com/aws/aws-cdk-go/awscdk/v2/awslakeformation"
	"github.com/aws/aws-cdk-go/awscdk/v2/awslambda"
	"github.com/aws/aws-cdk-go/awscdk/v2/awslambdaeventsources"
	"github.com/aws/aws-cdk-go/awscdk/v2/awslogs"
	"github.com/aws/aws-cdk-go/awscdk/v2/awss3"
	"github.com/aws/aws-cdk-go/awscdk/v2/awss3deployment"
	"github.com/aws/aws-cdk-go/awscdk/v2/awssqs"
	"github.com/aws/constructs-go/constructs/v10"
	"github.com/aws/jsii-runtime-go"
)

type LateRecomputationStackProps struct {
	awscdk.StackProps
	ErrorStreamArn         string
	ErrorStreamName        string
	SensorIdentityArn       string
	SensorIdentityName      string
	SensorIdentityStreamArn string
	TableBucket            string
}

func NewLateRecomputationStack(scope constructs.Construct, id string, props *LateRecomputationStackProps) awscdk.Stack {
	stack := awscdk.NewStack(scope, &id, &props.StackProps)
	region := *stack.Region()
	account := *stack.Account()

	// ── Glue script bucket + script deployment ──
	scriptBucket := awss3.NewBucket(stack, jsii.String("GlueScriptBucket"), &awss3.BucketProps{
		BucketName:        jsii.String("glue-scripts-" + account + "-" + region),
		RemovalPolicy:     awscdk.RemovalPolicy_DESTROY,
		AutoDeleteObjects: jsii.Bool(true),
	})
	awss3deployment.NewBucketDeployment(stack, jsii.String("DeployGlueScript"), &awss3deployment.BucketDeploymentProps{
		Sources:              &[]awss3deployment.ISource{awss3deployment.Source_Asset(jsii.String("./glue"), nil)},
		DestinationBucket:    scriptBucket,
		DestinationKeyPrefix: jsii.String("late-recomputation/"),
	})

	// ── Glue job IAM role ──
	managedPolicies := []awsiam.IManagedPolicy{
		awsiam.ManagedPolicy_FromAwsManagedPolicyName(jsii.String("service-role/AWSGlueServiceRole")),
		awsiam.ManagedPolicy_FromAwsManagedPolicyName(jsii.String("AmazonS3TablesFullAccess")),
		awsiam.ManagedPolicy_FromAwsManagedPolicyName(jsii.String("AWSLakeFormationDataAdmin")),
		awsiam.ManagedPolicy_FromAwsManagedPolicyName(jsii.String("AmazonS3FullAccess")),
	}
	glueRole := awsiam.NewRole(stack, jsii.String("GlueJobRole"), &awsiam.RoleProps{
		AssumedBy:       awsiam.NewServicePrincipal(jsii.String("glue.amazonaws.com"), nil),
		ManagedPolicies: &managedPolicies,
	})

	addPolicy := func(actions, resources *[]*string, conditions *map[string]interface{}) {
		stmt := &awsiam.PolicyStatementProps{
			Effect: awsiam.Effect_ALLOW, Actions: actions, Resources: resources,
		}
		if conditions != nil {
			stmt.Conditions = conditions
		}
		glueRole.AddToPolicy(awsiam.NewPolicyStatement(stmt))
	}

	addPolicy(
		jsii.Strings("glue:PassConnection",
			"lakeformation:RegisterResource",
			"lakeformation:RegisterResourceWithPrivilegedAccess"),
		jsii.Strings("*"), nil)

	addPolicy(
		jsii.Strings("iam:PassRole"),
		&[]*string{glueRole.RoleArn()},
		&map[string]interface{}{
			"StringEquals": map[string]interface{}{"iam:PassedToService": "glue.amazonaws.com"},
		})

	addPolicy(
		jsii.Strings("sts:AssumeRole"),
		jsii.Strings("arn:aws:iam::"+account+":role/aws-service-role/lakeformation.amazonaws.com/AWSServiceRoleForLakeFormationDataAccess"),
		nil)

	// ── Lake Formation permissions ──
	s3tablesCatalogId := account + ":s3tablescatalog/" + props.TableBucket
	dlPrincipal := &awslakeformation.CfnPermissions_DataLakePrincipalProperty{
		DataLakePrincipalIdentifier: glueRole.RoleArn(),
	}

	awslakeformation.NewCfnPermissions(stack, jsii.String("GlueLfLinkDbPermissions"), &awslakeformation.CfnPermissionsProps{
		DataLakePrincipal: dlPrincipal,
		Resource: &awslakeformation.CfnPermissions_ResourceProperty{
			DatabaseResource: &awslakeformation.CfnPermissions_DatabaseResourceProperty{
				Name: jsii.String("all_link"), CatalogId: jsii.String(account),
			},
		},
		Permissions: jsii.Strings("DESCRIBE"),
	})
	awslakeformation.NewCfnPermissions(stack, jsii.String("GlueLfDbPermissions"), &awslakeformation.CfnPermissionsProps{
		DataLakePrincipal: dlPrincipal,
		Resource: &awslakeformation.CfnPermissions_ResourceProperty{
			DatabaseResource: &awslakeformation.CfnPermissions_DatabaseResourceProperty{
				Name: jsii.String("all"), CatalogId: jsii.String(s3tablesCatalogId),
			},
		},
		Permissions: jsii.Strings("DESCRIBE"),
	})
	awslakeformation.NewCfnPermissions(stack, jsii.String("GlueLfRawDataPermissions"), &awslakeformation.CfnPermissionsProps{
		DataLakePrincipal: dlPrincipal,
		Resource: &awslakeformation.CfnPermissions_ResourceProperty{
			TableResource: &awslakeformation.CfnPermissions_TableResourceProperty{
				DatabaseName: jsii.String("all"), Name: jsii.String("raw_data"),
				CatalogId: jsii.String(s3tablesCatalogId),
			},
		},
		Permissions: jsii.Strings("SELECT", "DESCRIBE"),
	})
	awslakeformation.NewCfnPermissions(stack, jsii.String("GlueLfLogicalMeterPermissions"), &awslakeformation.CfnPermissionsProps{
		DataLakePrincipal: dlPrincipal,
		Resource: &awslakeformation.CfnPermissions_ResourceProperty{
			TableResource: &awslakeformation.CfnPermissions_TableResourceProperty{
				DatabaseName: jsii.String("all"), Name: jsii.String("logical_data"),
				CatalogId: jsii.String(s3tablesCatalogId),
			},
		},
		Permissions: jsii.Strings("SELECT", "ALTER", "DELETE", "DESCRIBE", "DROP", "INSERT"),
	})

	// Glue catalog access for S3 Tables
	addPolicy(
		jsii.Strings("glue:GetDatabase", "glue:GetDatabases", "glue:GetTable", "glue:GetTables", "glue:GetCatalog"),
		jsii.Strings(
			"arn:aws:glue:"+region+":"+account+":catalog",
			"arn:aws:glue:"+region+":"+account+":catalog/s3tablescatalog",
			"arn:aws:glue:"+region+":"+account+":catalog/s3tablescatalog/"+props.TableBucket,
			"arn:aws:glue:"+region+":"+account+":database/s3tablescatalog/"+props.TableBucket+"/*",
			"arn:aws:glue:"+region+":"+account+":table/s3tablescatalog/"+props.TableBucket+"/*/*",
		),
		nil)

	// DynamoDB read for meter identity
	addPolicy(jsii.Strings("dynamodb:Scan", "dynamodb:Query"), &[]*string{jsii.String(props.SensorIdentityArn)}, nil)
	scriptBucket.GrantRead(glueRole, nil)

	// ── Glue job ──
	awsglue.NewCfnJob(stack, jsii.String("LateRecomputationJob"), &awsglue.CfnJobProps{
		Name: jsii.String("late-data-recomputation"),
		Role: glueRole.RoleArn(),
		Command: &awsglue.CfnJob_JobCommandProperty{
			Name:           jsii.String("glueetl"),
			PythonVersion:  jsii.String("3"),
			ScriptLocation: jsii.String("s3://" + *scriptBucket.BucketName() + "/late-recomputation/late_recomputation.py"),
		},
		GlueVersion:     jsii.String("5.0"),
		WorkerType:      jsii.String("G.1X"),
		NumberOfWorkers: jsii.Number(2),
		Timeout:         jsii.Number(60),
		// Targeted backfills run on disjoint daq_ids and logical_meter_data is
		// event-sourced (newest ingested_time wins), so concurrent runs are safe.
		// Default is 1 → rapid attaches collided with ConcurrentRunsExceeded and
		// were dropped; allow several at once so every attach backfills.
		ExecutionProperty: &awsglue.CfnJob_ExecutionPropertyProperty{
			MaxConcurrentRuns: jsii.Number(10),
		},
		DefaultArguments: &map[string]string{
			"--region":                              region,
			"--sensor_identity_table":                props.SensorIdentityName,
			"--table_bucket_name":                   props.TableBucket,
			"--account_id":                          account,
			"--conf":                                "spark.sql.extensions=org.apache.iceberg.spark.extensions.IcebergSparkSessionExtensions",
			"--enable-glue-datacatalog":             "true",
			"--enable-metrics":                      "true",
			"--enable-continuous-cloudwatch-log":    "true",
			"--job-language":                        "python",
		},
	})

	// ── Late-arrival trigger Lambda (consumes error stream) ──
	triggerFn := awslambda.NewFunction(stack, jsii.String("LateArrivalTrigger"), &awslambda.FunctionProps{
		FunctionName: jsii.String("late-arrival-trigger"),
		Runtime:      awslambda.Runtime_PYTHON_3_12(),
		Handler:      jsii.String("handler.handler"),
		Code:         awslambda.Code_FromAsset(jsii.String("./lambda/late_arrival_trigger"), nil),
		Timeout:      awscdk.Duration_Minutes(jsii.Number(1)),
		MemorySize:   jsii.Number(128),
		Environment: &map[string]*string{
			"GLUE_JOB_NAME":        jsii.String("late-data-recomputation"),
			"REGION":               jsii.String(region),
			"SENSOR_IDENTITY_TABLE": jsii.String(props.SensorIdentityName),
			"TABLE_BUCKET_NAME":    jsii.String(props.TableBucket),
			"ACCOUNT_ID":           jsii.String(account),
		},
		LogRetention: awslogs.RetentionDays_ONE_WEEK,
	})

	errorStream := awskinesis.Stream_FromStreamArn(stack, jsii.String("ErrorStreamRef"), jsii.String(props.ErrorStreamArn))
	triggerFn.AddEventSource(awslambdaeventsources.NewKinesisEventSource(errorStream, &awslambdaeventsources.KinesisEventSourceProps{
		StartingPosition:  awslambda.StartingPosition_LATEST,
		BatchSize:         jsii.Number(100),
		MaxBatchingWindow: awscdk.Duration_Minutes(jsii.Number(5)),
		RetryAttempts:     jsii.Number(3),
	}))
	triggerFn.AddToRolePolicy(awsiam.NewPolicyStatement(&awsiam.PolicyStatementProps{
		Effect:  awsiam.Effect_ALLOW,
		Actions: jsii.Strings("glue:StartJobRun", "glue:GetJobRuns"),
		Resources: jsii.Strings(
			"arn:aws:glue:" + region + ":" + account + ":job/late-data-recomputation",
		),
	}))

	// ── Backfill trigger Lambda (DDB stream on sensor-identity, INSERT only) ──
	backfillFn := awslambda.NewFunction(stack, jsii.String("BackfillTrigger"), &awslambda.FunctionProps{
		FunctionName: jsii.String("backfill-trigger"),
		Runtime:      awslambda.Runtime_PYTHON_3_12(),
		Handler:      jsii.String("handler.handler"),
		Code:         awslambda.Code_FromAsset(jsii.String("./lambda/backfill_trigger"), nil),
		Timeout:      awscdk.Duration_Minutes(jsii.Number(1)),
		MemorySize:   jsii.Number(128),
		Environment: &map[string]*string{
			"GLUE_JOB_NAME":        jsii.String("late-data-recomputation"),
			"REGION":               jsii.String(region),
			"SENSOR_IDENTITY_TABLE": jsii.String(props.SensorIdentityName),
			"TABLE_BUCKET_NAME":    jsii.String(props.TableBucket),
			"ACCOUNT_ID":           jsii.String(account),
		},
		LogRetention: awslogs.RetentionDays_ONE_WEEK,
	})

	sensorIdTable := awsdynamodb.Table_FromTableAttributes(stack, jsii.String("SensorIdentityRef"),
		&awsdynamodb.TableAttributes{
			TableName:      jsii.String(props.SensorIdentityName),
			TableStreamArn: jsii.String(props.SensorIdentityStreamArn),
		})

	// DLQ backstop: anything that still can't be backfilled after the retry
	// window lands here (visible, not silently lost) instead of being discarded.
	backfillDlq := awssqs.NewQueue(stack, jsii.String("BackfillTriggerDlq"), &awssqs.QueueProps{
		QueueName:       jsii.String("backfill-trigger-dlq"),
		RetentionPeriod: awscdk.Duration_Days(jsii.Number(14)),
	})

	backfillFn.AddEventSource(awslambdaeventsources.NewDynamoEventSource(sensorIdTable, &awslambdaeventsources.DynamoEventSourceProps{
		StartingPosition:  awslambda.StartingPosition_LATEST,
		BatchSize:         jsii.Number(10),
		MaxBatchingWindow: awscdk.Duration_Seconds(jsii.Number(30)),
		// ConcurrentRunsExceeded is transient (a slot frees when a ~2 min Glue run
		// ends), so retry well past that window before giving up; bisect isolates a
		// genuinely-poison record; survivors go to the DLQ rather than vanishing.
		RetryAttempts:      jsii.Number(10),
		BisectBatchOnError: jsii.Bool(true),
		MaxRecordAge:       awscdk.Duration_Hours(jsii.Number(6)),
		OnFailure:          awslambdaeventsources.NewSqsDlq(backfillDlq),
		Filters: &[]*map[string]interface{}{
			{"pattern": `{"eventName":["INSERT"]}`},
		},
	}))
	backfillFn.AddToRolePolicy(awsiam.NewPolicyStatement(&awsiam.PolicyStatementProps{
		Effect:  awsiam.Effect_ALLOW,
		Actions: jsii.Strings("glue:StartJobRun", "glue:GetJobRuns"),
		Resources: jsii.Strings(
			"arn:aws:glue:" + region + ":" + account + ":job/late-data-recomputation",
		),
	}))

	return stack
}
