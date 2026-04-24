package main

import (
	"os"

	"github.com/aws/aws-cdk-go/awscdk/v2"
	"github.com/aws/aws-cdk-go/awscdk/v2/awscognito"
	"github.com/aws/aws-cdk-go/awscdk/v2/awsssm"
	"github.com/aws/constructs-go/constructs/v10"
	"github.com/aws/jsii-runtime-go"
)

type CognitoStackProps struct {
	awscdk.StackProps
}

func NewCognitoStack(scope constructs.Construct, id string, props *CognitoStackProps) awscdk.Stack {
	var sprops awscdk.StackProps
	if props != nil {
		sprops = props.StackProps
	}
	stack := awscdk.NewStack(scope, &id, &sprops)

	// TODO: Consider backup!
	// https://repost.aws/questions/QUI37vTJrbQr2ZdFaF9gX9pw/is-there-a-way-to-create-backups-of-cognito-user-pools

	// Create Cognito User Pool
	userPool := awscognito.NewUserPool(stack, jsii.String("OcamlUserPool"), &awscognito.UserPoolProps{
		UserPoolName:      jsii.String("OcamlUserPool"),
		SelfSignUpEnabled: jsii.Bool(false), // Disable self sign-up, admin creates users
		SignInAliases: &awscognito.SignInAliases{
			Email: jsii.Bool(true),
		},
		AutoVerify: &awscognito.AutoVerifiedAttrs{
			Email: jsii.Bool(true),
		},
		StandardAttributes: &awscognito.StandardAttributes{
			Email: &awscognito.StandardAttribute{
				Required: jsii.Bool(true),
				Mutable:  jsii.Bool(true),
			},
			GivenName: &awscognito.StandardAttribute{
				Required: jsii.Bool(false),
				Mutable:  jsii.Bool(true),
			},
			FamilyName: &awscognito.StandardAttribute{
				Required: jsii.Bool(false),
				Mutable:  jsii.Bool(true),
			},
		},
		PasswordPolicy: &awscognito.PasswordPolicy{
			MinLength:        jsii.Number(6),
			RequireLowercase: jsii.Bool(true),
			RequireUppercase: jsii.Bool(true),
			RequireDigits:    jsii.Bool(true),
			RequireSymbols:   jsii.Bool(false),
		},
		AccountRecovery:    awscognito.AccountRecovery_EMAIL_AND_PHONE_WITHOUT_MFA,
		DeletionProtection: jsii.Bool(true), // LEAVE THIS TRUE!
		// Disable email verification requirement for admin-created users
		UserVerification: &awscognito.UserVerificationConfig{
			EmailStyle: awscognito.VerificationEmailStyle_CODE,
		},
	})

	// Create Cognito User Pool Client for user authentication
	userPoolClient := awscognito.NewUserPoolClient(stack, jsii.String("OcamlUserPoolClient"), &awscognito.UserPoolClientProps{
		UserPool:           userPool,
		UserPoolClientName: jsii.String("OcamlUserPoolClient"),
		AuthFlows: &awscognito.AuthFlow{
			UserPassword:      jsii.Bool(true),
			UserSrp:           jsii.Bool(true),
			AdminUserPassword: jsii.Bool(true),
			Custom:            jsii.Bool(true),
		},
		EnableTokenRevocation:      jsii.Bool(true),
		PreventUserExistenceErrors: jsii.Bool(true),
		GenerateSecret:             jsii.Bool(false), // Important for public clients
		IdTokenValidity:            awscdk.Duration_Hours(jsii.Number(8)),
		AccessTokenValidity:        awscdk.Duration_Hours(jsii.Number(8)),
		RefreshTokenValidity:       awscdk.Duration_Days(jsii.Number(30)),
		OAuth: &awscognito.OAuthSettings{
			Flows: &awscognito.OAuthFlows{
				AuthorizationCodeGrant: jsii.Bool(true),
				ImplicitCodeGrant:      jsii.Bool(true),
			},
			Scopes: &[]awscognito.OAuthScope{
				awscognito.OAuthScope_OPENID(),
				awscognito.OAuthScope_EMAIL(),
				awscognito.OAuthScope_PROFILE(),
				awscognito.OAuthScope_COGNITO_ADMIN(),
				awscognito.OAuthScope_PHONE(),
			},
			CallbackUrls: &[]*string{
				jsii.String("http://localhost:4321/"),
				jsii.String("http://localhost:4321/main"),
				jsii.String("https://d24beiqs2cj89y.cloudfront.net/"),
				jsii.String("https://d24beiqs2cj89y.cloudfront.net/main"),
			},
			LogoutUrls: &[]*string{
				jsii.String("http://localhost:4321/"),
				jsii.String("https://d24beiqs2cj89y.cloudfront.net/"),
			},
		},
	})

	awsssm.NewStringParameter(stack, jsii.String("OcamlCognitoUserPoolIdParameter"), &awsssm.StringParameterProps{
		ParameterName: jsii.String("/cognito/ocaml-user-pool-id"),
		StringValue:   userPool.UserPoolId(),
		Description:   jsii.String("OCaml Cognito User Pool ID"),
	})

	awsssm.NewStringParameter(stack, jsii.String("OcamlUserPoolClientIdParameter"), &awsssm.StringParameterProps{
		ParameterName: jsii.String("/cognito/ocaml-user-pool-client-id"),
		StringValue:   userPoolClient.UserPoolClientId(),
		Description:   jsii.String("OCaml User Pool Client ID"),
	})

	awscdk.NewCfnOutput(stack, jsii.String("UserPoolId"), &awscdk.CfnOutputProps{
		Value: userPool.UserPoolId(),
	})
	awscdk.NewCfnOutput(stack, jsii.String("UserPoolClientId"), &awscdk.CfnOutputProps{
		Value: userPoolClient.UserPoolClientId(),
	})

	return stack
}

func main() {
	defer jsii.Close()

	app := awscdk.NewApp(nil)

	NewCognitoStack(app, "OcamlCognitoStack", &CognitoStackProps{
		awscdk.StackProps{
			Env: env(),
			Description: jsii.String("OCaml EMS Cognito User Pool"),
		},
	})

	app.Synth(nil)
}

// env determines the AWS environment (account+region) in which our stack is to
// be deployed. For more information see: https://docs.aws.amazon.com/cdk/latest/guide/environments.html
func env() *awscdk.Environment {
	return &awscdk.Environment{
		Account: jsii.String(os.Getenv("CDK_DEFAULT_ACCOUNT")),
		Region:  jsii.String(os.Getenv("CDK_DEFAULT_REGION")),
	}
}
