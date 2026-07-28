package main

import (
	"github.com/aws/aws-cdk-go/awscdk/v2"
	"github.com/aws/aws-cdk-go/awscdk/v2/awsapigatewayv2"
	"github.com/aws/aws-cdk-go/awscdk/v2/awsapigatewayv2integrations"
	"github.com/aws/aws-cdk-go/awscdk/v2/awsdynamodb"
	"github.com/aws/aws-cdk-go/awscdk/v2/awsglue"
	"github.com/aws/aws-cdk-go/awscdk/v2/awsiam"
	"github.com/aws/aws-cdk-go/awscdk/v2/awslakeformation"
	"github.com/aws/aws-cdk-go/awscdk/v2/awslambda"
	"github.com/aws/aws-cdk-go/awscdk/v2/awslogs"
	"github.com/aws/aws-cdk-go/awscdk/v2/awss3"
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

	// gsi1: company rollups keyed by aggregation dimension, so a cross-resource
	// company view ("all energy") is one partition query. gsi1pk = "HN2#<id>#<energy|volume>",
	// gsi1sk = "<node_path>#<gran>#<bucket>" (resource omitted ⇒ spans resources, date last
	// for clean time ranges). Adding a GSI is an online UpdateTable — non-destructive.
	table.AddGlobalSecondaryIndex(&awsdynamodb.GlobalSecondaryIndexProps{
		IndexName:      jsii.String("gsi1"),
		PartitionKey:   &awsdynamodb.Attribute{Name: jsii.String("gsi1pk"), Type: awsdynamodb.AttributeType_STRING},
		SortKey:        &awsdynamodb.Attribute{Name: jsii.String("gsi1sk"), Type: awsdynamodb.AttributeType_STRING},
		ProjectionType: awsdynamodb.ProjectionType_ALL,
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
	// Explicitly named: the hierarchy account's HierarchyReaderRole trusts this ARN
	// cross-account. A CFN-generated name carries a random suffix, so any future role
	// replacement would silently break the roll-up's read of the coefficient matrix.
	glueRole := awsiam.NewRole(stack, jsii.String("AggGlueJobRole"), &awsiam.RoleProps{
		RoleName:        jsii.String("MeasurementsAggregateGlueRole"),
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
	// Cross-account: read the materialised coefficient matrix out of hierarchy_new
	// in the hierarchy account (spec §8). The role there trusts this role by name.
	glueRole.AddToPolicy(awsiam.NewPolicyStatement(&awsiam.PolicyStatementProps{
		Effect:    awsiam.Effect_ALLOW,
		Actions:   jsii.Strings("sts:AssumeRole"),
		Resources: jsii.Strings("arn:aws:iam::339712745226:role/HierarchyReaderRole"),
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
	// The job prunes rollup rows it no longer produces (a formula change can make a
	// purpose's rows vanish), which means reading the partition back before deleting.
	glueRole.AddToPolicy(awsiam.NewPolicyStatement(&awsiam.PolicyStatementProps{
		Effect:    awsiam.Effect_ALLOW,
		Actions:   jsii.Strings("dynamodb:Query"),
		Resources: &[]*string{table.TableArn()},
	}))
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
				DatabaseName: jsii.String("all"), Name: jsii.String("logical_data"),
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
			"--region":            region,
			"--table_bucket_name": props.TableBucket,
			"--account_id":        account,
			"--rollup_table":      *table.TableName(),
			"--lookback_days":     props.LookbackDays,
			// Cross-account read of the coefficient matrix in hierarchy_new. The job
			// assumes this; the sts:AssumeRole grant is on glueRole above.
			"--hierarchy_reader_role_arn": "arn:aws:iam::339712745226:role/HierarchyReaderRole",
			// hierarchy_matrix.py is imported by measurements_aggregate.py at runtime and
			// must be shipped alongside the script, not just deployed to the bucket.
			"--extra-py-files":                   "s3://" + *scriptBucket.BucketName() + "/measurements-aggregate/hierarchy_matrix.py",
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

	// ── Rust / Graviton (arm64) read API — the single production read lambda. ──
	// Two routes on ONE Function URL (PUBLIC_AGG_API_BASE_URL): `/aggregations`
	// (JSON, Resource-Insights chart, reads DynamoDB) and `/measurements` (HTML
	// fragment, Datatilegnelse page, reads `all.raw_data` via **Amazon Athena** —
	// Athena does the dedup GROUP BY server-side; ~3.5s vs ~12s for iceberg-direct).
	// Kept to one lambda to limit the Datadog-instrumented function count.
	// Built via `cargo lambda build --release --arm64 -p aggregations`.
	athenaResults := "daq-athena-query-results-" + account + "-" + region
	aggFn := awslambda.NewFunction(stack, jsii.String("AggregationsFn"), &awslambda.FunctionProps{
		FunctionName: jsii.String("measurements-aggregations-api"),
		Runtime:      awslambda.Runtime_PROVIDED_AL2023(),
		Architecture: awslambda.Architecture_ARM_64(),
		Handler:      jsii.String("bootstrap"),
		Code:         awslambda.Code_FromAsset(jsii.String("../../../target/lambda/aggregations"), nil),
		Timeout:      awscdk.Duration_Seconds(jsii.Number(30)),
		MemorySize:   jsii.Number(512),
		Environment: &map[string]*string{
			"ROLLUP_TABLE":     table.TableName(),
			"ATHENA_WORKGROUP": jsii.String("daq-workgroup"),
			"ATHENA_OUTPUT":    jsii.String("s3://" + athenaResults + "/"),
			"ATHENA_CATALOG":   jsii.String("s3tablescatalog/" + props.TableBucket),
			"ATHENA_DATABASE":  jsii.String("all"),
			"ATHENA_TABLE":     jsii.String("raw_data"),
		},
		LogRetention: awslogs.RetentionDays_ONE_WEEK,
	})
	table.GrantReadData(aggFn)

	// The `/measurements` route queries `all.raw_data` via Athena: needs athena query
	// exec on the workgroup, Glue catalog read for the s3tables federated catalog, R/W
	// on the Athena results bucket, lakeformation:GetDataAccess, and (below) Lake
	// Formation SELECT on the table. NOTE: first-deploy-verify — if the runtime hits
	// AccessDenied, add the missing action.
	aggFn.AddToRolePolicy(awsiam.NewPolicyStatement(&awsiam.PolicyStatementProps{
		Effect:    awsiam.Effect_ALLOW,
		Actions:   jsii.Strings("athena:StartQueryExecution", "athena:GetQueryExecution", "athena:GetQueryResults", "athena:StopQueryExecution"),
		Resources: jsii.Strings("arn:aws:athena:" + region + ":" + account + ":workgroup/daq-workgroup"),
	}))
	aggFn.AddToRolePolicy(awsiam.NewPolicyStatement(&awsiam.PolicyStatementProps{
		Effect:  awsiam.Effect_ALLOW,
		Actions: jsii.Strings("glue:GetDatabase", "glue:GetDatabases", "glue:GetTable", "glue:GetTables", "glue:GetCatalog", "glue:GetPartitions"),
		Resources: jsii.Strings(
			"arn:aws:glue:"+region+":"+account+":catalog",
			"arn:aws:glue:"+region+":"+account+":catalog/s3tablescatalog",
			"arn:aws:glue:"+region+":"+account+":catalog/s3tablescatalog/"+props.TableBucket,
			"arn:aws:glue:"+region+":"+account+":database/s3tablescatalog/"+props.TableBucket+"/*",
			"arn:aws:glue:"+region+":"+account+":table/s3tablescatalog/"+props.TableBucket+"/*/*",
		),
	}))
	aggFn.AddToRolePolicy(awsiam.NewPolicyStatement(&awsiam.PolicyStatementProps{
		Effect:    awsiam.Effect_ALLOW,
		Actions:   jsii.Strings("s3:GetBucketLocation", "s3:GetObject", "s3:PutObject", "s3:ListBucket", "s3:ListMultipartUploadParts", "s3:AbortMultipartUpload"),
		Resources: jsii.Strings("arn:aws:s3:::"+athenaResults, "arn:aws:s3:::"+athenaResults+"/*"),
	}))
	aggFn.AddToRolePolicy(awsiam.NewPolicyStatement(&awsiam.PolicyStatementProps{
		Effect:    awsiam.Effect_ALLOW,
		Actions:   jsii.Strings("lakeformation:GetDataAccess"),
		Resources: jsii.Strings("*"),
	}))
	aggDl := &awslakeformation.CfnPermissions_DataLakePrincipalProperty{
		DataLakePrincipalIdentifier: aggFn.Role().RoleArn(),
	}
	awslakeformation.NewCfnPermissions(stack, jsii.String("RawLfDbPermissions"), &awslakeformation.CfnPermissionsProps{
		DataLakePrincipal: aggDl,
		Resource: &awslakeformation.CfnPermissions_ResourceProperty{
			DatabaseResource: &awslakeformation.CfnPermissions_DatabaseResourceProperty{
				Name: jsii.String("all"), CatalogId: jsii.String(s3tablesCatalogId),
			},
		},
		Permissions: jsii.Strings("DESCRIBE"),
	})
	awslakeformation.NewCfnPermissions(stack, jsii.String("RawLfTablePermissions"), &awslakeformation.CfnPermissionsProps{
		DataLakePrincipal: aggDl,
		Resource: &awslakeformation.CfnPermissions_ResourceProperty{
			TableResource: &awslakeformation.CfnPermissions_TableResourceProperty{
				DatabaseName: jsii.String("all"), Name: jsii.String("raw_data"),
				CatalogId: jsii.String(s3tablesCatalogId),
			},
		},
		Permissions: jsii.Strings("SELECT", "DESCRIBE"),
	})
	// get_liveness reads all.heartbeat via Athena — needs its own LF SELECT grant.
	awslakeformation.NewCfnPermissions(stack, jsii.String("HeartbeatLfTablePermissions"), &awslakeformation.CfnPermissionsProps{
		DataLakePrincipal: aggDl,
		Resource: &awslakeformation.CfnPermissions_ResourceProperty{
			TableResource: &awslakeformation.CfnPermissions_TableResourceProperty{
				DatabaseName: jsii.String("all"), Name: jsii.String("heartbeat"),
				CatalogId: jsii.String(s3tablesCatalogId),
			},
		},
		Permissions: jsii.Strings("SELECT", "DESCRIBE"),
	})

	// Public read API on an API Gateway HTTP API (replaces the earlier Function URL,
	// to match the hierarchy service and gain access logs / throttling / WAF / custom
	// domains / a future Cognito JWT authorizer). CQRS query side only (read-only
	// lambda — meter data is written by the Flink/Glue pipeline, not here), mirroring
	// the hierarchy `GET /query/{action}` convention:
	//   GET /meterdata/query/{action}  (action = get_measurements | get_aggregations)
	//   GET /meterdata/openapi.json
	//   GET /meterdata/docs
	// Explicit routes (not a `/{proxy+}` catch-all) so the gateway owns the surface
	// (per-route metrics / throttle / authorizers) and rejects unknown paths itself.
	// All GET, so the gateway auto-answers the OPTIONS CORS preflight (a catch-all
	// `ANY` would instead hand OPTIONS to the lambda → 404 → preflight fails). The
	// lambda emits no CORS (`api::Cors::None`); `CorsPreflight` adds it.
	// `HttpLambdaIntegration` adds the lambda invoke permission automatically.
	aggApi := awsapigatewayv2.NewHttpApi(stack, jsii.String("AggregationsHttpApi"), &awsapigatewayv2.HttpApiProps{
		ApiName: jsii.String("measurements-aggregations-api"),
		CorsPreflight: &awsapigatewayv2.CorsPreflightOptions{
			AllowOrigins: jsii.Strings("*"),
			AllowMethods: &[]awsapigatewayv2.CorsHttpMethod{
				awsapigatewayv2.CorsHttpMethod_GET, awsapigatewayv2.CorsHttpMethod_OPTIONS,
			},
			AllowHeaders: jsii.Strings("*"),
		},
	})
	aggInteg := awsapigatewayv2integrations.NewHttpLambdaIntegration(
		jsii.String("AggregationsIntegration"), aggFn, &awsapigatewayv2integrations.HttpLambdaIntegrationProps{},
	)
	for _, p := range []string{"/meterdata/query/{action}", "/meterdata/openapi.json", "/meterdata/docs"} {
		aggApi.AddRoutes(&awsapigatewayv2.AddRoutesOptions{
			Path:        jsii.String(p),
			Methods:     &[]awsapigatewayv2.HttpMethod{awsapigatewayv2.HttpMethod_GET},
			Integration: aggInteg,
		})
	}
	awscdk.NewCfnOutput(stack, jsii.String("AggregationsApiUrl"), &awscdk.CfnOutputProps{
		Value:       aggApi.Url(),
		Description: jsii.String("Public HTTP API base — GET /aggregations (chart JSON) + GET /measurements (Datatilegnelse HTML) + GET /openapi.json"),
	})

	return stack
}
