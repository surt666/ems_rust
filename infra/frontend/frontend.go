package main

import (
	"os"

	"github.com/aws/aws-cdk-go/awscdk/v2"
	"github.com/aws/aws-cdk-go/awscdk/v2/awscloudfront"
	"github.com/aws/aws-cdk-go/awscdk/v2/awscloudfrontorigins"
	"github.com/aws/aws-cdk-go/awscdk/v2/awss3"
	"github.com/aws/aws-cdk-go/awscdk/v2/awss3deployment"
	"github.com/aws/aws-cdk-go/awscdk/v2/awsssm"
	"github.com/aws/constructs-go/constructs/v10"
	"github.com/aws/jsii-runtime-go"
)

type FrontendStackProps struct {
	awscdk.StackProps
}

func NewFrontendStack(scope constructs.Construct, id string, props *FrontendStackProps) awscdk.Stack {
	var sprops awscdk.StackProps
	if props != nil {
		sprops = props.StackProps
	}
	stack := awscdk.NewStack(scope, &id, &sprops)

	websiteBucket := awss3.NewBucket(stack, jsii.String("EmsFrontendBucket"), &awss3.BucketProps{
		PublicReadAccess: jsii.Bool(true),
		BlockPublicAccess: awss3.NewBlockPublicAccess(&awss3.BlockPublicAccessOptions{
			BlockPublicAcls:       jsii.Bool(true),
			IgnorePublicAcls:      jsii.Bool(true),
			BlockPublicPolicy:     jsii.Bool(false),
			RestrictPublicBuckets: jsii.Bool(false),
		}),
		WebsiteIndexDocument: jsii.String("index.html"),
		WebsiteErrorDocument: jsii.String("index.html"),
		RemovalPolicy:        awscdk.RemovalPolicy_RETAIN,
		AutoDeleteObjects:    jsii.Bool(false),
		Versioned:            jsii.Bool(false),
	})

	// Pull the OCaml hierarchy API Gateway URL from SSM (set by OcamlHierarchyStack).
	apiUrlParam := awsssm.StringParameter_FromStringParameterName(
		stack,
		jsii.String("OcamlHierarchyApiUrlParameter"),
		jsii.String("/api/ocaml-hierarchy-api-url"),
	)
	apiUrl := apiUrlParam.StringValue()
	apiDomain := awscdk.Fn_Select(jsii.Number(1), awscdk.Fn_Split(jsii.String("://"), apiUrl, nil))
	apiDomainClean := awscdk.Fn_Select(jsii.Number(0), awscdk.Fn_Split(jsii.String("/"), apiDomain, nil))

	// One cache policy reused for every API path — pass-through, no caching.
	apiCachePolicy := awscloudfront.NewCachePolicy(stack, jsii.String("OcamlHierarchyApiCachePolicy"), &awscloudfront.CachePolicyProps{
		CachePolicyName:            jsii.String("OcamlHierarchyApiCachePolicy"),
		Comment:                    jsii.String("Pass-through cache policy for OCaml hierarchy API"),
		DefaultTtl:                 awscdk.Duration_Seconds(jsii.Number(0)),
		MinTtl:                     awscdk.Duration_Seconds(jsii.Number(0)),
		MaxTtl:                     awscdk.Duration_Seconds(jsii.Number(1)),
		QueryStringBehavior:        awscloudfront.CacheQueryStringBehavior_All(),
		HeaderBehavior:             awscloudfront.CacheHeaderBehavior_None(),
		CookieBehavior:             awscloudfront.CacheCookieBehavior_None(),
		EnableAcceptEncodingGzip:   jsii.Bool(false),
		EnableAcceptEncodingBrotli: jsii.Bool(false),
	})

	apiOrigin := awscloudfrontorigins.NewHttpOrigin(apiDomainClean, &awscloudfrontorigins.HttpOriginProps{
		ProtocolPolicy: awscloudfront.OriginProtocolPolicy_HTTPS_ONLY,
	})

	apiBehavior := &awscloudfront.BehaviorOptions{
		Origin:                apiOrigin,
		ViewerProtocolPolicy:  awscloudfront.ViewerProtocolPolicy_REDIRECT_TO_HTTPS,
		AllowedMethods:        awscloudfront.AllowedMethods_ALLOW_ALL(),
		CachePolicy:           apiCachePolicy,
		OriginRequestPolicy:   awscloudfront.OriginRequestPolicy_ALL_VIEWER_EXCEPT_HOST_HEADER(),
		ResponseHeadersPolicy: awscloudfront.ResponseHeadersPolicy_CORS_ALLOW_ALL_ORIGINS(),
	}

	distribution := awscloudfront.NewDistribution(stack, jsii.String("EmsFrontendDistribution"), &awscloudfront.DistributionProps{
		Comment:           jsii.String("EMS OCaml Frontend Distribution"),
		DefaultRootObject: jsii.String("index.html"),
		DefaultBehavior: &awscloudfront.BehaviorOptions{
			Origin:               awscloudfrontorigins.NewS3StaticWebsiteOrigin(websiteBucket, nil),
			ViewerProtocolPolicy: awscloudfront.ViewerProtocolPolicy_REDIRECT_TO_HTTPS,
			AllowedMethods:       awscloudfront.AllowedMethods_ALLOW_GET_HEAD_OPTIONS(),
			CachedMethods:        awscloudfront.CachedMethods_CACHE_GET_HEAD_OPTIONS(),
			Compress:             jsii.Bool(true),
			CachePolicy:          awscloudfront.CachePolicy_CACHING_OPTIMIZED(),
		},
		AdditionalBehaviors: &map[string]*awscloudfront.BehaviorOptions{
			"/command":         apiBehavior,
			"/query/*":         apiBehavior,
			"/hierarchy/*":     apiBehavior,
		},
		PriceClass: awscloudfront.PriceClass_PRICE_CLASS_100,
	})

	awss3deployment.NewBucketDeployment(stack, jsii.String("DeployFrontend"), &awss3deployment.BucketDeploymentProps{
		Sources: &[]awss3deployment.ISource{
			awss3deployment.Source_Asset(jsii.String("../../frontend/dist"), nil),
		},
		DestinationBucket: websiteBucket,
		Distribution:      distribution,
		DistributionPaths: &[]*string{jsii.String("/*")},
	})

	awsssm.NewStringParameter(stack, jsii.String("CloudFrontUrlParameter"), &awsssm.StringParameterProps{
		ParameterName: jsii.String("/frontend/cloudfront-url"),
		StringValue:   jsii.String("https://" + *distribution.DistributionDomainName()),
		Description:   jsii.String("CloudFront Distribution URL for frontend"),
	})

	awscdk.NewCfnOutput(stack, jsii.String("DistributionDomainName"), &awscdk.CfnOutputProps{
		Value: distribution.DistributionDomainName(),
	})

	return stack
}

func main() {
	defer jsii.Close()

	app := awscdk.NewApp(nil)

	NewFrontendStack(app, "EmsFrontendStack", &FrontendStackProps{
		awscdk.StackProps{
			Env:         env(),
			Description: jsii.String("EMS Frontend - S3 + CloudFront fronting the OCaml hierarchy API"),
		},
	})

	app.Synth(nil)
}

func env() *awscdk.Environment {
	return &awscdk.Environment{
		Account: jsii.String(os.Getenv("CDK_DEFAULT_ACCOUNT")),
		Region:  jsii.String(os.Getenv("CDK_DEFAULT_REGION")),
	}
}
