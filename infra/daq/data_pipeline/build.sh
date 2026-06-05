#!/usr/bin/env bash
set -euo pipefail

# Build the Scala Flink assembly JAR
cd ./flink_app_scala
sbt clean assembly
cd ..

# Deploy with CDK (Go entrypoint via cdk.json)
npx cdk deploy DaqPipelineStack LateRecomputationStack OcamlBridgeWriterRoleStack \
  --require-approval never \
  -c SHA="$(git rev-parse --short HEAD)" \
  -c RUN_NR="$(date +%s)" \
  -c ParPerKPU=1 \
  -c MaxKPU=4 \
  -c TableBucketName=measurements
