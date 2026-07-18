package main

import (
	"github.com/aws/aws-cdk-go/awscdk/v2"
	"github.com/aws/aws-cdk-go/awscdk/v2/awsapigatewayv2"
	"github.com/aws/aws-cdk-go/awscdk/v2/awsapigatewayv2integrations"
	"github.com/aws/aws-cdk-go/awscdk/v2/awsiam"
	"github.com/aws/aws-cdk-go/awscdk/v2/awslakeformation"
	"github.com/aws/aws-cdk-go/awscdk/v2/awslambda"
	"github.com/aws/aws-cdk-go/awscdk/v2/awslogs"
	"github.com/aws/constructs-go/constructs/v10"
	"github.com/aws/jsii-runtime-go"
)

type QueryRawStackProps struct {
	awscdk.StackProps
	TableBucket string
}

// NewQueryRawStack deploys the `query-raw-duck` lambda: a DuckDB reader of
// `all.raw_data` (S3 Tables / Iceberg) that serves the Datatilegnelse measurements
// page (`GET /meterdata/query/get_measurements`). It won an engine benchmark against a
// DataFusion + iceberg-rust arm (2026-07-17, ~1.3s warm vs ~5s + a 25s wide-window
// cliff); that arm has been removed. (Stack + resource names keep the "QueryRaw" prefix
// to avoid CFN replacement churn.)
//
// IAM: iceberg talks to the native S3 Tables API (not the Glue federated catalog), so
// it needs `s3tables:*` (AmazonS3TablesFullAccess), `lakeformation:GetDataAccess`, and a
// Lake Formation SELECT/DESCRIBE grant on all.raw_data for the lambda role.
//
// Build first: crates/services/query-raw-duck/build-in-al2023.sh (the Code asset reads
// ../../../target/lambda/query-raw-duck; the DuckDB extensions need a GNU-toolchain
// build — see that script + the fnDuck comment).
func NewQueryRawStack(scope constructs.Construct, id string, props *QueryRawStackProps) awscdk.Stack {
	stack := awscdk.NewStack(scope, &id, &props.StackProps)
	region := *stack.Region()
	account := *stack.Account()

	tableBucketArn := "arn:aws:s3tables:" + region + ":" + account + ":bucket/" + props.TableBucket

	// DuckDB reader of all.raw_data (S3 Tables / Iceberg) — the production Datatilegnelse
	// backend (~1.3s warm; beat the DataFusion arm which was benchmarked then torn down).
	// **x86_64 + AL2023-toolchain build**: the DuckDB extension .so's are GCC-built and
	// need GNU libstdc++, so the bootstrap must use the same runtime — a zig/cargo-lambda
	// build statically embeds LLVM libc++ and ABI-crashes on extension LOAD. We ship
	// AL2023's libstdc++.so.6 + libgcc_s.so.1 under lib/ with LD_LIBRARY_PATH. Extensions
	// INSTALL at runtime into /tmp (non-VPC lambda has internet — bundling them regressed
	// cold start, so we don't). Build: crates/services/query-raw-duck/build-in-al2023.sh
	fnDuck := awslambda.NewFunction(stack, jsii.String("QueryRawDuckFn"), &awslambda.FunctionProps{
		FunctionName: jsii.String("measurements-query-raw-duck"),
		Runtime:      awslambda.Runtime_PROVIDED_AL2023(),
		Architecture: awslambda.Architecture_X86_64(),
		Handler:      jsii.String("bootstrap"),
		Code:         awslambda.Code_FromAsset(jsii.String("../../../target/lambda/query-raw-duck"), nil),
		Timeout:      awscdk.Duration_Seconds(jsii.Number(60)),
		MemorySize:   jsii.Number(3008),
		Environment: &map[string]*string{
			"TABLE_BUCKET_ARN": jsii.String(tableBucketArn),
			// Device-liveness lake (plain Parquet, read via DuckDB read_parquet). Written by
			// the meter-heartbeat lambda; stable name so no cross-stack import is needed.
			"HEARTBEAT_BUCKET": jsii.String("meter-heartbeat-" + account + "-" + region),
			// Prepend our bundled libs so the DuckDB extension's libstdc++ resolves.
			"LD_LIBRARY_PATH": jsii.String("/var/task/lib:/var/runtime/lib:/var/lang/lib:/lib64:/usr/lib64:/opt/lib"),
		},
		LogRetention: awslogs.RetentionDays_ONE_WEEK,
	})
	grantRawDataAccess(stack, "QueryRawDuck", account, props.TableBucket, fnDuck.Role())

	// Read the heartbeat Parquet lake (device-status route). Plain S3 GET/LIST — the
	// bucket is owned by MeterHeartbeatStack; referenced by its stable name.
	heartbeatBucketArn := "arn:aws:s3:::meter-heartbeat-" + account + "-" + region
	fnDuck.Role().AddToPrincipalPolicy(awsiam.NewPolicyStatement(&awsiam.PolicyStatementProps{
		Effect:    awsiam.Effect_ALLOW,
		Actions:   jsii.Strings("s3:GetObject", "s3:ListBucket"),
		Resources: jsii.Strings(heartbeatBucketArn, heartbeatBucketArn+"/*"),
	}))

	// Public HTTP API — read-only GET routes (query-string params, fixed paths).
	// GET+OPTIONS lets the gateway answer the CORS preflight itself.
	api := awsapigatewayv2.NewHttpApi(stack, jsii.String("QueryRawHttpApi"), &awsapigatewayv2.HttpApiProps{
		ApiName: jsii.String("measurements-query-raw"),
		CorsPreflight: &awsapigatewayv2.CorsPreflightOptions{
			AllowOrigins: jsii.Strings("*"),
			AllowMethods: &[]awsapigatewayv2.CorsHttpMethod{
				awsapigatewayv2.CorsHttpMethod_GET, awsapigatewayv2.CorsHttpMethod_OPTIONS,
			},
			AllowHeaders: jsii.Strings("*"),
		},
	})
	integDuck := awsapigatewayv2integrations.NewHttpLambdaIntegration(
		jsii.String("QueryRawDuckIntegration"), fnDuck, &awsapigatewayv2integrations.HttpLambdaIntegrationProps{},
	)
	// Production Datatilegnelse route (same path as the aggregations lambda, different
	// HTTP API/base) so the frontend only swaps its base URL. Plus a raw JSON route
	// kept for ad-hoc debugging.
	for _, p := range []string{"/meterdata/query/get_measurements", "/rawdata/query-duck", "/rawdevice/status"} {
		api.AddRoutes(&awsapigatewayv2.AddRoutesOptions{
			Path:        jsii.String(p),
			Methods:     &[]awsapigatewayv2.HttpMethod{awsapigatewayv2.HttpMethod_GET},
			Integration: integDuck,
		})
	}

	awscdk.NewCfnOutput(stack, jsii.String("QueryRawApiUrl"), &awscdk.CfnOutputProps{
		Value:       api.Url(),
		Description: jsii.String("query-raw-duck HTTP API base — GET /meterdata/query/get_measurements (Datatilegnelse) + /rawdata/query-duck (raw JSON)"),
	})

	return stack
}

// grantRawDataAccess wires the S3 Tables native-API access (AmazonS3TablesFullAccess
// managed policy), lakeformation:GetDataAccess, and a Lake Formation SELECT/DESCRIBE
// grant on all.raw_data for one lambda role. Both engine arms use identical access;
// idPrefix keeps the CFN logical IDs unique per lambda.
func grantRawDataAccess(stack awscdk.Stack, idPrefix, account, tableBucket string, role awsiam.IRole) {
	role.AddManagedPolicy(
		awsiam.ManagedPolicy_FromAwsManagedPolicyName(jsii.String("AmazonS3TablesFullAccess")),
	)
	role.AddToPrincipalPolicy(awsiam.NewPolicyStatement(&awsiam.PolicyStatementProps{
		Effect:    awsiam.Effect_ALLOW,
		Actions:   jsii.Strings("lakeformation:GetDataAccess"),
		Resources: jsii.Strings("*"),
	}))

	s3tablesCatalogId := account + ":s3tablescatalog/" + tableBucket
	dl := &awslakeformation.CfnPermissions_DataLakePrincipalProperty{
		DataLakePrincipalIdentifier: role.RoleArn(),
	}
	awslakeformation.NewCfnPermissions(stack, jsii.String(idPrefix+"LfDbPermissions"), &awslakeformation.CfnPermissionsProps{
		DataLakePrincipal: dl,
		Resource: &awslakeformation.CfnPermissions_ResourceProperty{
			DatabaseResource: &awslakeformation.CfnPermissions_DatabaseResourceProperty{
				Name: jsii.String("all"), CatalogId: jsii.String(s3tablesCatalogId),
			},
		},
		Permissions: jsii.Strings("DESCRIBE"),
	})
	awslakeformation.NewCfnPermissions(stack, jsii.String(idPrefix+"LfTablePermissions"), &awslakeformation.CfnPermissionsProps{
		DataLakePrincipal: dl,
		Resource: &awslakeformation.CfnPermissions_ResourceProperty{
			TableResource: &awslakeformation.CfnPermissions_TableResourceProperty{
				DatabaseName: jsii.String("all"), Name: jsii.String("raw_data"),
				CatalogId: jsii.String(s3tablesCatalogId),
			},
		},
		Permissions: jsii.Strings("SELECT", "DESCRIBE"),
	})
}
