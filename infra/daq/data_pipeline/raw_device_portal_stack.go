package main

import (
	"github.com/aws/aws-cdk-go/awscdk/v2"
	"github.com/aws/aws-cdk-go/awscdk/v2/awscloudfront"
	"github.com/aws/aws-cdk-go/awscdk/v2/awscloudfrontorigins"
	"github.com/aws/aws-cdk-go/awscdk/v2/awss3"
	"github.com/aws/aws-cdk-go/awscdk/v2/awss3deployment"
	"github.com/aws/constructs-go/constructs/v10"
	"github.com/aws/jsii-runtime-go"
)

type RawDevicePortalStackProps struct {
	awscdk.StackProps
	// ShellOrigin is the main app's CloudFront origin, allowed to hx-get this MFE.
	ShellOrigin string
}

// NewRawDevicePortalStack hosts the Raw Device htmx microfrontend: a private S3 bucket
// behind CloudFront (HTTPS/OAC), with a CORS response-headers policy so the main app
// shell can compose it via a cross-origin hx-get + hx-select. Demo → bucket DESTROYs.
func NewRawDevicePortalStack(scope constructs.Construct, id string, props *RawDevicePortalStackProps) awscdk.Stack {
	stack := awscdk.NewStack(scope, &id, &props.StackProps)

	bucket := awss3.NewBucket(stack, jsii.String("RawDeviceBucket"), &awss3.BucketProps{
		RemovalPolicy:     awscdk.RemovalPolicy_DESTROY,
		AutoDeleteObjects: jsii.Bool(true),
		BlockPublicAccess: awss3.BlockPublicAccess_BLOCK_ALL(),
	})

	// CORS so the HTTPS shell's cross-origin hx-get is allowed. noHeaders on the client
	// keeps it a simple GET (no preflight), so the response header alone suffices.
	cors := awscloudfront.NewResponseHeadersPolicy(stack, jsii.String("RawDeviceCors"), &awscloudfront.ResponseHeadersPolicyProps{
		CorsBehavior: &awscloudfront.ResponseHeadersCorsBehavior{
			AccessControlAllowOrigins:     jsii.Strings(props.ShellOrigin),
			AccessControlAllowMethods:     jsii.Strings("GET"),
			AccessControlAllowHeaders:     jsii.Strings("*"),
			AccessControlAllowCredentials: jsii.Bool(false),
			OriginOverride:                jsii.Bool(true),
		},
	})

	dist := awscloudfront.NewDistribution(stack, jsii.String("RawDeviceDistribution"), &awscloudfront.DistributionProps{
		DefaultRootObject: jsii.String("index.html"),
		DefaultBehavior: &awscloudfront.BehaviorOptions{
			Origin:                awscloudfrontorigins.S3BucketOrigin_WithOriginAccessControl(bucket, nil),
			ViewerProtocolPolicy:  awscloudfront.ViewerProtocolPolicy_REDIRECT_TO_HTTPS,
			ResponseHeadersPolicy: cors,
		},
	})

	awss3deployment.NewBucketDeployment(stack, jsii.String("DeployRawDevice"), &awss3deployment.BucketDeploymentProps{
		Sources:           &[]awss3deployment.ISource{awss3deployment.Source_Asset(jsii.String("../../../frontend-raw-device"), nil)},
		DestinationBucket: bucket,
		Distribution:      dist,
		DistributionPaths: jsii.Strings("/*"),
	})

	awscdk.NewCfnOutput(stack, jsii.String("RawDeviceMfeUrl"), &awscdk.CfnOutputProps{
		Value:       jsii.String("https://" + *dist.DistributionDomainName()),
		Description: jsii.String("Raw Device MFE base URL — set as PUBLIC_RAWDEVICE_MFE_URL in the shell"),
	})

	return stack
}
