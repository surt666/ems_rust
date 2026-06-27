# rsbench — Redshift Spectrum `/measurements` backend (manual deploy, **no CDK**)

VPC (tier2) Rust lambda that queries **Redshift Serverless** (Spectrum over the
`all.raw_data` S3 Tables table) via `tokio-postgres`, returning the **same HTML
fragment** as the Athena `/measurements` route. Selectable from the frontend's
**"Redshift (VPC)"** checkbox on `/measurements`. Lives in the **daq** account
(891377204778); deployed by hand (intentionally not in the CDK).

## AWS resources (daq_dev, eu-central-1)
- Redshift Serverless: namespace `spectrum-spike-ns`, workgroup `spectrum-spike-wg`
  (8 RPU, tier2 subnets `subnet-05f66bfe68a1e9f75`/`…0a62ba450e8457234`, **no EVR** → 2-AZ ok)
- Redshift role `redshift-spectrum-spike-role` + LF SELECT on `all.raw_data`;
  Glue resource link `raw_rl`; Redshift external schema `spectrum_rl`
- Lambda `rsbench`: arm64, VPC SG `sg-0a2e9b46282e4e05f`, role `rsbench-lambda-role`,
  env `REDSHIFT_HOST/USER/PASSWORD/DB`
- API Gateway HTTP API `fpweiie2z5` → `rsbench`, CORS `GET *`
  (`https://fpweiie2z5.execute-api.eu-central-1.amazonaws.com`)
- Frontend `PUBLIC_REDSHIFT_API_BASE_URL` = that API URL

## Build + deploy (manual)
```
cargo lambda build --release --arm64
( cd target/lambda/rsbench && zip -j /tmp/rsbench.zip bootstrap )
aws lambda update-function-code --function-name rsbench \
  --zip-file fileb:///tmp/rsbench.zip --profile daq_dev --region eu-central-1
```

## Notes
- Latency: warm ~1.8s, **cold ~16s** after Redshift Serverless auto-pauses.
- The Redshift admin password is in the lambda env (spike). Move to Secrets Manager for production.
