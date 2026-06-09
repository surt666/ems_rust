// Cognito mirror for the hierarchy user directory (account 339). Triggered by the
// hierarchy_new DynamoDB stream, filtered to type=user rows:
//
//	INSERT/MODIFY  -> AdminCreateUser(email) + AdminAddUserToGroup(cognito_group)
//	REMOVE         -> AdminDeleteUser(email)
//
// The OCaml create_user command writes the user row (+ access grants) to DynamoDB; this
// keeps the Cognito user pool in sync. Idempotent (ignores already-exists / not-found),
// reports per-record failures so only the bad ones retry / hit the DLQ.
package main

import (
	"context"
	"errors"
	"log"
	"os"
	"strings"

	"github.com/aws/aws-lambda-go/events"
	"github.com/aws/aws-lambda-go/lambda"
	"github.com/aws/aws-sdk-go-v2/aws"
	"github.com/aws/aws-sdk-go-v2/config"
	cip "github.com/aws/aws-sdk-go-v2/service/cognitoidentityprovider"
	ciptypes "github.com/aws/aws-sdk-go-v2/service/cognitoidentityprovider/types"
)

var (
	idp        *cip.Client
	userPoolID = os.Getenv("USER_POOL_ID")
)

// emailFromPK turns the user item's pk ("U#<email>") into the email.
func emailFromPK(img map[string]events.DynamoDBAttributeValue) string {
	if v, ok := img["pk"]; ok {
		return strings.TrimPrefix(v.String(), "U#")
	}
	return ""
}

func strAttr(img map[string]events.DynamoDBAttributeValue, key string) string {
	if v, ok := img[key]; ok {
		return v.String()
	}
	return ""
}

func isUser(img map[string]events.DynamoDBAttributeValue) bool {
	return strAttr(img, "type") == "user"
}

// ensureUser creates the Cognito user (invite email) and puts them in the group.
func ensureUser(ctx context.Context, email, group string) error {
	if email == "" {
		return nil
	}
	_, err := idp.AdminCreateUser(ctx, &cip.AdminCreateUserInput{
		UserPoolId: aws.String(userPoolID),
		Username:   aws.String(email),
		UserAttributes: []ciptypes.AttributeType{
			{Name: aws.String("email"), Value: aws.String(email)},
			{Name: aws.String("email_verified"), Value: aws.String("true")},
		},
	})
	if err != nil {
		var exists *ciptypes.UsernameExistsException
		if !errors.As(err, &exists) {
			return err
		}
		log.Printf("user %s already exists, ensuring group only", email)
	}
	if group == "" {
		return nil
	}
	_, err = idp.AdminAddUserToGroup(ctx, &cip.AdminAddUserToGroupInput{
		UserPoolId: aws.String(userPoolID),
		Username:   aws.String(email),
		GroupName:  aws.String(group),
	})
	return err
}

func deleteUser(ctx context.Context, email string) error {
	if email == "" {
		return nil
	}
	_, err := idp.AdminDeleteUser(ctx, &cip.AdminDeleteUserInput{
		UserPoolId: aws.String(userPoolID),
		Username:   aws.String(email),
	})
	var notFound *ciptypes.UserNotFoundException
	if errors.As(err, &notFound) {
		return nil
	}
	return err
}

func handleRecord(ctx context.Context, rec events.DynamoDBEventRecord) error {
	switch rec.EventName {
	case "INSERT", "MODIFY":
		img := rec.Change.NewImage
		if !isUser(img) {
			return nil
		}
		return ensureUser(ctx, emailFromPK(img), strAttr(img, "cognito_group"))
	case "REMOVE":
		img := rec.Change.OldImage
		if !isUser(img) {
			return nil
		}
		return deleteUser(ctx, emailFromPK(img))
	default:
		return nil
	}
}

func handler(ctx context.Context, e events.DynamoDBEvent) (events.DynamoDBEventResponse, error) {
	var failures []events.DynamoDBBatchItemFailure
	for _, rec := range e.Records {
		if err := handleRecord(ctx, rec); err != nil {
			log.Printf("record %s failed: %v", rec.EventID, err)
			failures = append(failures, events.DynamoDBBatchItemFailure{ItemIdentifier: rec.EventID})
		}
	}
	return events.DynamoDBEventResponse{BatchItemFailures: failures}, nil
}

func main() {
	cfg, err := config.LoadDefaultConfig(context.Background())
	if err != nil {
		panic(err)
	}
	idp = cip.NewFromConfig(cfg)
	lambda.Start(handler)
}
