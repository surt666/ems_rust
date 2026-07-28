package main

import (
	"github.com/aws/aws-cdk-go/awscdk/v2/awsapigatewayv2authorizers"
	"github.com/aws/jsii-runtime-go"
)

// Cognito lives in the hierarchy account; a JWT authorizer only needs the public
// issuer/JWKS URL, so the DAQ-account APIs validate against it with no
// cross-account IAM. Rotating the app client is a one-line edit here rather than
// a hunt through two stacks.
const (
	cognitoIssuer   = "https://cognito-idp.eu-central-1.amazonaws.com/eu-central-1_gADB2vK24"
	cognitoAudience = "2fidjt2pmacepu39h4nhqcv0h1"
)

// newCognitoJwtAuthorizer builds the authorizer these APIs put on their data
// routes. Documentation routes (openapi.json, docs) stay open: a browser opening
// Swagger UI cannot present a token, and they describe the surface rather than
// serve data. This is authentication only — per-user scoping is a later concern.
func newCognitoJwtAuthorizer(id string) awsapigatewayv2authorizers.HttpJwtAuthorizer {
	return awsapigatewayv2authorizers.NewHttpJwtAuthorizer(
		jsii.String(id),
		jsii.String(cognitoIssuer),
		&awsapigatewayv2authorizers.HttpJwtAuthorizerProps{
			AuthorizerName: jsii.String("cognito-jwt"),
			JwtAudience:    jsii.Strings(cognitoAudience),
		},
	)
}
