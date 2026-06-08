package main

import (
	"github.com/aws/aws-cdk-go/awscdk/v2"
	"github.com/aws/aws-cdk-go/awscdk/v2/awsdynamodb"
	"github.com/aws/aws-cdk-go/awscdk/v2/awsglue"
	"github.com/aws/aws-cdk-go/awscdk/v2/awsiam"
	"github.com/aws/aws-cdk-go/awscdk/v2/awslakeformation"
	"github.com/aws/aws-cdk-go/awscdk/v2/awslambda"
	"github.com/aws/aws-cdk-go/awscdk/v2/awslogs"
	"github.com/aws/aws-cdk-go/awscdk/v2/awss3"
	"github.com/aws/aws-cdk-go/awscdk/v2/awss3assets"
	"github.com/aws/aws-cdk-go/awscdk/v2/awss3deployment"
	"github.com/aws/constructs-go/constructs/v10"
	"github.com/aws/jsii-runtime-go"
)

type MeasurementsAggregateStackProps struct {
	awscdk.StackProps
	TableBucket  string
	LookbackDays string
}

func NewMeasurementsAggregateStack(scope constructs.Construct, id string, props *MeasurementsAggregateStackProps) awscdk.Stack {
	stack := awscdk.NewStack(scope, &id, &props.StackProps)
	region := *stack.Region()
	account := *stack.Account()

	// ── DynamoDB materialized-view table (on-demand, TTL, RETAIN) ──
	table := awsdynamodb.NewTable(stack, jsii.String("MeasurementsAggregate"), &awsdynamodb.TableProps{
		TableName:           jsii.String("measurements_aggregate"),
		PartitionKey:        &awsdynamodb.Attribute{Name: jsii.String("pk"), Type: awsdynamodb.AttributeType_STRING},
		SortKey:             &awsdynamodb.Attribute{Name: jsii.String("sk"), Type: awsdynamodb.AttributeType_STRING},
		BillingMode:         awsdynamodb.BillingMode_PAY_PER_REQUEST,
		TimeToLiveAttribute: jsii.String("ttl"),
		RemovalPolicy:       awscdk.RemovalPolicy_RETAIN,
	})

	// ── Glue script bucket + deploy ./glue under measurements-aggregate/ ──
	scriptBucket := awss3.NewBucket(stack, jsii.String("AggScriptBucket"), &awss3.BucketProps{
		BucketName:        jsii.String("glue-agg-scripts-" + account + "-" + region),
		RemovalPolicy:     awscdk.RemovalPolicy_DESTROY,
		AutoDeleteObjects: jsii.Bool(true),
	})
	awss3deployment.NewBucketDeployment(stack, jsii.String("DeployAggScript"), &awss3deployment.BucketDeploymentProps{
		Sources:              &[]awss3deployment.ISource{awss3deployment.Source_Asset(jsii.String("./glue"), nil)},
		DestinationBucket:    scriptBucket,
		DestinationKeyPrefix: jsii.String("measurements-aggregate/"),
	})

	// ── Glue job IAM role (mirror late_recomputation_stack.go managed policies) ──
	managedPolicies := []awsiam.IManagedPolicy{
		awsiam.ManagedPolicy_FromAwsManagedPolicyName(jsii.String("service-role/AWSGlueServiceRole")),
		awsiam.ManagedPolicy_FromAwsManagedPolicyName(jsii.String("AmazonS3TablesFullAccess")),
		awsiam.ManagedPolicy_FromAwsManagedPolicyName(jsii.String("AWSLakeFormationDataAdmin")),
		awsiam.ManagedPolicy_FromAwsManagedPolicyName(jsii.String("AmazonS3FullAccess")),
	}
	glueRole := awsiam.NewRole(stack, jsii.String("AggGlueJobRole"), &awsiam.RoleProps{
		AssumedBy:       awsiam.NewServicePrincipal(jsii.String("glue.amazonaws.com"), nil),
		ManagedPolicies: &managedPolicies,
	})

	// Lake Formation registration / passrole / assume-role inline policies
	// (mirror late_recomputation_stack.go).
	glueRole.AddToPolicy(awsiam.NewPolicyStatement(&awsiam.PolicyStatementProps{
		Effect: awsiam.Effect_ALLOW,
		Actions: jsii.Strings("glue:PassConnection",
			"lakeformation:RegisterResource",
			"lakeformation:RegisterResourceWithPrivilegedAccess"),
		Resources: jsii.Strings("*"),
	}))
	glueRole.AddToPolicy(awsiam.NewPolicyStatement(&awsiam.PolicyStatementProps{
		Effect:    awsiam.Effect_ALLOW,
		Actions:   jsii.Strings("iam:PassRole"),
		Resources: &[]*string{glueRole.RoleArn()},
		Conditions: &map[string]interface{}{
			"StringEquals": map[string]interface{}{"iam:PassedToService": "glue.amazonaws.com"},
		},
	}))
	glueRole.AddToPolicy(awsiam.NewPolicyStatement(&awsiam.PolicyStatementProps{
		Effect:    awsiam.Effect_ALLOW,
		Actions:   jsii.Strings("sts:AssumeRole"),
		Resources: jsii.Strings("arn:aws:iam::" + account + ":role/aws-service-role/lakeformation.amazonaws.com/AWSServiceRoleForLakeFormationDataAccess"),
	}))

	// Glue catalog access for S3 Tables
	glueRole.AddToPolicy(awsiam.NewPolicyStatement(&awsiam.PolicyStatementProps{
		Effect:  awsiam.Effect_ALLOW,
		Actions: jsii.Strings("glue:GetDatabase", "glue:GetDatabases", "glue:GetTable", "glue:GetTables", "glue:GetCatalog"),
		Resources: jsii.Strings(
			"arn:aws:glue:"+region+":"+account+":catalog",
			"arn:aws:glue:"+region+":"+account+":catalog/s3tablescatalog",
			"arn:aws:glue:"+region+":"+account+":catalog/s3tablescatalog/"+props.TableBucket,
			"arn:aws:glue:"+region+":"+account+":database/s3tablescatalog/"+props.TableBucket+"/*",
			"arn:aws:glue:"+region+":"+account+":table/s3tablescatalog/"+props.TableBucket+"/*/*",
		),
	}))
	table.GrantWriteData(glueRole)
	scriptBucket.GrantRead(glueRole, nil)

	// ── Lake Formation permissions ──
	s3tablesCatalogId := account + ":s3tablescatalog/" + props.TableBucket
	dlPrincipal := &awslakeformation.CfnPermissions_DataLakePrincipalProperty{
		DataLakePrincipalIdentifier: glueRole.RoleArn(),
	}
	awslakeformation.NewCfnPermissions(stack, jsii.String("AggLfLinkDbPermissions"), &awslakeformation.CfnPermissionsProps{
		DataLakePrincipal: dlPrincipal,
		Resource: &awslakeformation.CfnPermissions_ResourceProperty{
			DatabaseResource: &awslakeformation.CfnPermissions_DatabaseResourceProperty{
				Name: jsii.String("all_link"), CatalogId: jsii.String(account),
			},
		},
		Permissions: jsii.Strings("DESCRIBE"),
	})
	awslakeformation.NewCfnPermissions(stack, jsii.String("AggLfDbPermissions"), &awslakeformation.CfnPermissionsProps{
		DataLakePrincipal: dlPrincipal,
		Resource: &awslakeformation.CfnPermissions_ResourceProperty{
			DatabaseResource: &awslakeformation.CfnPermissions_DatabaseResourceProperty{
				Name: jsii.String("all"), CatalogId: jsii.String(s3tablesCatalogId),
			},
		},
		Permissions: jsii.Strings("DESCRIBE"),
	})
	awslakeformation.NewCfnPermissions(stack, jsii.String("AggLfTablePermissions"), &awslakeformation.CfnPermissionsProps{
		DataLakePrincipal: dlPrincipal,
		Resource: &awslakeformation.CfnPermissions_ResourceProperty{
			TableResource: &awslakeformation.CfnPermissions_TableResourceProperty{
				DatabaseName: jsii.String("all"), Name: jsii.String("logical_meter_data"),
				CatalogId: jsii.String(s3tablesCatalogId),
			},
		},
		Permissions: jsii.Strings("SELECT", "DESCRIBE"),
	})

	// ── Glue job ──
	awsglue.NewCfnJob(stack, jsii.String("MeasurementsAggregateJob"), &awsglue.CfnJobProps{
		Name: jsii.String("measurements-aggregate"),
		Role: glueRole.RoleArn(),
		Command: &awsglue.CfnJob_JobCommandProperty{
			Name:           jsii.String("glueetl"),
			PythonVersion:  jsii.String("3"),
			ScriptLocation: jsii.String("s3://" + *scriptBucket.BucketName() + "/measurements-aggregate/measurements_aggregate.py"),
		},
		GlueVersion:     jsii.String("5.0"),
		WorkerType:      jsii.String("G.1X"),
		NumberOfWorkers: jsii.Number(2),
		Timeout:         jsii.Number(60),
		DefaultArguments: &map[string]string{
			"--region":                           region,
			"--table_bucket_name":                props.TableBucket,
			"--account_id":                       account,
			"--rollup_table":                     *table.TableName(),
			"--lookback_days":                    props.LookbackDays,
			"--conf":                             "spark.sql.extensions=org.apache.iceberg.spark.extensions.IcebergSparkSessionExtensions",
			"--enable-glue-datacatalog":          "true",
			"--enable-metrics":                   "true",
			"--enable-continuous-cloudwatch-log": "true",
			"--job-language":                     "python",
		},
	})

	// ── Hourly schedule via native Glue scheduled trigger (5 min past the hour) ──
	awsglue.NewCfnTrigger(stack, jsii.String("AggHourlyTrigger"), &awsglue.CfnTriggerProps{
		Name:            jsii.String("measurements-aggregate-hourly"),
		Type:            jsii.String("SCHEDULED"),
		Schedule:        jsii.String("cron(5 * * * ? *)"),
		StartOnCreation: jsii.Bool(true),
		Actions: &[]interface{}{
			&awsglue.CfnTrigger_ActionProperty{JobName: jsii.String("measurements-aggregate")},
		},
	})

	// ── Read API: Lambda behind a public Function URL (Resource Insights chart) ──
	aggFn := awslambda.NewFunction(stack, jsii.String("AggregationsFn"), &awslambda.FunctionProps{
		FunctionName: jsii.String("measurements-aggregations-api"),
		Runtime:      awslambda.Runtime_PYTHON_3_12(),
		Handler:      jsii.String("handler.handler"),
		Code: awslambda.Code_FromAsset(jsii.String("./lambda/aggregations"),
			&awss3assets.AssetOptions{Exclude: jsii.Strings("test_*.py", "__pycache__")}),
		Timeout:      awscdk.Duration_Seconds(jsii.Number(30)),
		MemorySize:   jsii.Number(256),
		Environment:  &map[string]*string{"ROLLUP_TABLE": table.TableName()},
		LogRetention: awslogs.RetentionDays_ONE_WEEK,
	})
	table.GrantReadData(aggFn)

	aggUrl := aggFn.AddFunctionUrl(&awslambda.FunctionUrlOptions{
		AuthType: awslambda.FunctionUrlAuthType_NONE,
		Cors: &awslambda.FunctionUrlCorsOptions{
			AllowedOrigins: jsii.Strings("*"),
			AllowedMethods: &[]awslambda.HttpMethod{awslambda.HttpMethod_GET},
			AllowedHeaders: jsii.Strings("*"),
		},
	})
	awscdk.NewCfnOutput(stack, jsii.String("AggregationsUrl"), &awscdk.CfnOutputProps{
		Value:       aggUrl.Url(),
		Description: jsii.String("Public Function URL for GET /aggregations (Resource Insights chart)"),
	})

	return stack
}
