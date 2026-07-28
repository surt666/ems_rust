package main

import (
	"github.com/aws/aws-cdk-go/awscdk/v2"
	"github.com/aws/aws-cdk-go/awscdk/v2/awsiam"
	"github.com/aws/constructs-go/constructs/v10"
	"github.com/aws/jsii-runtime-go"
)

const (
	emsAccountB    = "339712745226" // ems_ocaml: hierarchy_new
	bridgeRoleName = "OcamlBridgeWriterRole"
)

type OcamlBridgeWriterRoleStackProps struct {
	awscdk.StackProps
	SensorIdentityTableArn string
}

// Account A: IAM role assumed by the ems_ocaml bridge Lambda (deployed from
// ems_ocaml/infra/hierarchy/app.go) to upsert/delete `sensor-identity` rows.
func NewOcamlBridgeWriterRoleStack(scope constructs.Construct, id string, props *OcamlBridgeWriterRoleStackProps) awscdk.Stack {
	stack := awscdk.NewStack(scope, &id, &props.StackProps)

	role := awsiam.NewRole(stack, jsii.String("WriterRole"), &awsiam.RoleProps{
		RoleName:    jsii.String(bridgeRoleName),
		AssumedBy:   awsiam.NewAccountPrincipal(jsii.String(emsAccountB)),
		Description: jsii.String("Assumed by ems_ocaml bridge Lambda to upsert sensor-identity"),
	})

	role.AddToPolicy(awsiam.NewPolicyStatement(&awsiam.PolicyStatementProps{
		Effect:    awsiam.Effect_ALLOW,
		Actions:   jsii.Strings("dynamodb:PutItem", "dynamodb:DeleteItem", "dynamodb:UpdateItem"),
		Resources: jsii.Strings(props.SensorIdentityTableArn),
	}))

	awscdk.NewCfnOutput(stack, jsii.String("WriterRoleArn"), &awscdk.CfnOutputProps{Value: role.RoleArn()})
	return stack
}
