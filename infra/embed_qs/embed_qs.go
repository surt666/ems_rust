package main

import (
	"os"

	"github.com/aws/aws-cdk-go/awscdk/v2"
	"github.com/aws/aws-cdk-go/awscdk/v2/awsiam"
	"github.com/aws/aws-cdk-go/awscdk/v2/awslambda"
	"github.com/aws/constructs-go/constructs/v10"
	"github.com/aws/jsii-runtime-go"
)

type EmbedQsStackProps struct {
	awscdk.StackProps
}

func NewEmbedQsStack(scope constructs.Construct, id string, props *EmbedQsStackProps) awscdk.Stack {
	var sprops awscdk.StackProps
	if props != nil {
		sprops = props.StackProps
	}
	stack := awscdk.NewStack(scope, &id, &sprops)

	// Inline Python code
	pythonCode := `import json
import boto3
quicksight = boto3.client('quicksight', region_name='eu-central-1')
def lambda_handler(event, context):
    params = event.get('queryStringParameters', {})
    dashboard_id = params.get('dashboardId')

    user_arn = 'arn:aws:quicksight:eu-central-1:891377204778:user/default/AWSReservedSSO_AWSAdministratorAccess_0e6eb7a82b4057de/stel@enity.io'

    response = quicksight.generate_embed_url_for_registered_user(
        AwsAccountId='891377204778',
        UserArn=user_arn,
        ExperienceConfiguration={
            'Dashboard': {
                'InitialDashboardId': dashboard_id
            }
        },
        AllowedDomains=['http://localhost:4321', 'https://d368wcanc53tdl.cloudfront.net', 'https://d24beiqs2cj89y.cloudfront.net'],
        SessionLifetimeInMinutes=600
    )

    return {
        'statusCode': 200,
        'headers': {
            'Content-Type': 'application/json'
        },
        'body': json.dumps({'EmbedUrl': response['EmbedUrl']})
    }
`

	// Create Lambda function
	lambdaFn := awslambda.NewFunction(stack, jsii.String("QuickSightEmbedFunction"), &awslambda.FunctionProps{
		Runtime: awslambda.Runtime_PYTHON_3_12(),
		Handler: jsii.String("index.lambda_handler"),
		Code:    awslambda.Code_FromInline(jsii.String(pythonCode)),
		Timeout: awscdk.Duration_Seconds(jsii.Number(30)),
		Environment: &map[string]*string{
			"QUICKSIGHT_ACCOUNT_ID": jsii.String("891377204778"),
		},
	})

	// Add QuickSight permissions to Lambda
	lambdaFn.AddToRolePolicy(awsiam.NewPolicyStatement(&awsiam.PolicyStatementProps{
		Effect: awsiam.Effect_ALLOW,
		Actions: jsii.Strings(
			"quicksight:GenerateEmbedUrlForRegisteredUser",
			"quicksight:GetAuthCode",
			"quicksight:DescribeUser",
			"quicksight:ListUsers",
		),
		Resources: jsii.Strings("*"),
	}))

	// Create Lambda Function URL
	fnUrl := lambdaFn.AddFunctionUrl(&awslambda.FunctionUrlOptions{
		AuthType: awslambda.FunctionUrlAuthType_NONE,
		Cors: &awslambda.FunctionUrlCorsOptions{
			AllowedOrigins: jsii.Strings("*"),
			AllowedMethods: &[]awslambda.HttpMethod{
				awslambda.HttpMethod_GET,
			},
			AllowedHeaders: jsii.Strings("*"),
			MaxAge:         awscdk.Duration_Minutes(jsii.Number(5)),
		},
	})

	// Output the Function URL
	awscdk.NewCfnOutput(stack, jsii.String("FunctionUrl"), &awscdk.CfnOutputProps{
		Value:       fnUrl.Url(),
		Description: jsii.String("QuickSight Embed Lambda Function URL"),
	})

	return stack
}

func main() {
	defer jsii.Close()

	app := awscdk.NewApp(nil)

	NewEmbedQsStack(app, "EmbedQsStack", &EmbedQsStackProps{
		awscdk.StackProps{
			Env: env(),
		},
	})

	app.Synth(nil)
}

// env determines the AWS environment (account+region) in which our stack is to be deployed.
func env() *awscdk.Environment {
	return &awscdk.Environment{
		Account: jsii.String(os.Getenv("CDK_DEFAULT_ACCOUNT")),
		Region:  jsii.String(os.Getenv("CDK_DEFAULT_REGION")),
	}
}
