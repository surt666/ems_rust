package main

import (
	"os"
	"strconv"
	"time"

	"github.com/aws/aws-cdk-go/awscdk/v2"
	"github.com/aws/jsii-runtime-go"
)

func main() {
	defer jsii.Close()

	app := awscdk.NewApp(nil)

	sha := contextString(app, "SHA", "")
	parPerKPU := contextString(app, "ParPerKPU", "1")
	maxKPU := contextString(app, "MaxKPU", "1")
	tableBucketName := contextString(app, "TableBucketName", "measurements")
	runNr := contextString(app, "RUN_NR", strconv.FormatInt(time.Now().Unix(), 10))
	lookbackDays := contextString(app, "LookbackDays", "1")

	pipeline := NewDaqPipelineStack(app, "DaqPipelineStack", &DaqPipelineStackProps{
		StackProps: awscdk.StackProps{Env: defaultEnv()},
		AppName:    "flink-iceberg-processor",
		ParPerKPU:  parPerKPU,
		MaxKPUs:    maxKPU,
		Sha:        sha,
		TableBucket: tableBucketName,
	})

	lateRecomp := NewLateRecomputationStack(app, "LateRecomputationStack", &LateRecomputationStackProps{
		StackProps:        awscdk.StackProps{Env: defaultEnv()},
		ErrorStreamArn:    *pipeline.ErrorStream.StreamArn(),
		ErrorStreamName:   *pipeline.ErrorStream.StreamName(),
		MeterIdentityArn:  *pipeline.MeterIdentityTable.TableArn(),
		MeterIdentityName: *pipeline.MeterIdentityTable.TableName(),
		MeterIdentityStreamArn: *pipeline.MeterIdentityTable.TableStreamArn(),
		TableBucket:       tableBucketName,
	})
	lateRecomp.AddDependency(pipeline.Stack, jsii.String("LateRecomputation depends on meter-identity / error stream"))

	NewMeasurementsAggregateStack(app, "MeasurementsAggregateStack", &MeasurementsAggregateStackProps{
		StackProps:   awscdk.StackProps{Env: defaultEnv()},
		TableBucket:  tableBucketName,
		LookbackDays: lookbackDays,
	})

	// DuckDB reader of all.raw_data serving the Datatilegnelse page — its own stack so
	// it touches neither the RETAIN rollup table nor the aggregations lambda. (Won an
	// engine benchmark vs a since-removed DataFusion/iceberg-rust arm.)
	NewQueryRawStack(app, "QueryRawStack", &QueryRawStackProps{
		StackProps:  awscdk.StackProps{Env: defaultEnv()},
		TableBucket: tableBucketName,
	})

	// Raw Device htmx microfrontend (S3 + CloudFront) — composed into the main app shell.
	NewRawDevicePortalStack(app, "RawDevicePortalStack", &RawDevicePortalStackProps{
		StackProps:  awscdk.StackProps{Env: defaultEnv()},
		ShellOrigin: "https://d24beiqs2cj89y.cloudfront.net",
	})

	NewS3TablesStack(app, "S3TablesStack", &awscdk.StackProps{
		Env: &awscdk.Environment{
			Account: jsii.String("891377204778"),
			Region:  jsii.String("eu-central-1"),
		},
	})

	// Account A side of the ems_ocaml bridge: just the writer role.
	// The Lambda + DDB stream subscription live in ems_ocaml/infra/hierarchy/app.go.
	NewOcamlBridgeWriterRoleStack(app, "OcamlBridgeWriterRoleStack", &OcamlBridgeWriterRoleStackProps{
		StackProps:           awscdk.StackProps{Env: emsAccountEnv()},
		MeterIdentityTableArn: *pipeline.MeterIdentityTable.TableArn(),
	})

	awscdk.Tags_Of(pipeline.Stack).Add(jsii.String("version"), jsii.String(runNr), nil)
	awscdk.Tags_Of(pipeline.Stack).Add(jsii.String("git_sha"), jsii.String(sha), nil)

	app.Synth(nil)
}

func defaultEnv() *awscdk.Environment {
	return &awscdk.Environment{
		Account: jsii.String(os.Getenv("CDK_DEFAULT_ACCOUNT")),
		Region:  jsii.String(os.Getenv("CDK_DEFAULT_REGION")),
	}
}

// emsAccountEnv pins the writer-role stack to account A so a stray
// CDK_DEFAULT_ACCOUNT (e.g. when deploying from account B) doesn't
// silently move it.
func emsAccountEnv() *awscdk.Environment {
	return &awscdk.Environment{
		Account: jsii.String("891377204778"),
		Region:  jsii.String("eu-central-1"),
	}
}

func contextString(app awscdk.App, key, fallback string) string {
	v := app.Node().TryGetContext(jsii.String(key))
	if v == nil {
		return fallback
	}
	if s, ok := v.(string); ok && s != "" {
		return s
	}
	return fallback
}
