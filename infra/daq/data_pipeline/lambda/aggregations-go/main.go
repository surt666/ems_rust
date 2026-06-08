// Go port of lambda/aggregations/handler.py — same /aggregations read API over the
// measurements_aggregate DynamoDB view, built for Graviton (arm64) to compare against the
// Python version. Behind a public Lambda Function URL.
//
//	GET /aggregations?level_id=<hierarchy path>&resolution=<hourly|daily>&purpose=<opt>&start=<ISO>&end=<ISO>
//
// Response: JSON array, one row per (purpose, bucket), sorted by purpose then time.
package main

import (
	"cmp"
	"context"
	"encoding/json"
	"fmt"
	"os"
	"regexp"
	"sort"
	"strings"
	"time"

	"github.com/aws/aws-lambda-go/events"
	"github.com/aws/aws-lambda-go/lambda"
	"github.com/aws/aws-sdk-go-v2/aws"
	"github.com/aws/aws-sdk-go-v2/config"
	"github.com/aws/aws-sdk-go-v2/feature/dynamodb/attributevalue"
	"github.com/aws/aws-sdk-go-v2/feature/dynamodb/expression"
	"github.com/aws/aws-sdk-go-v2/service/dynamodb"
)

var (
	hnRe        = regexp.MustCompile(`HN(\d+)#(\d+)`)
	rollupTable = cmp.Or(os.Getenv("ROLLUP_TABLE"), "measurements_aggregate")
)

// ── pure helpers ──

// parseNodeKeys extracts the hierarchy nodes from HN2 down out of a frontend node path.
// Returns (pk, skPath): pk = "HN2#<id>", skPath = the full '|'-joined path from HN2.
// Errors if there is no HN2 (company) segment.
func parseNodeKeys(levelID string) (pk, skPath string, err error) {
	matches := hnRe.FindAllStringSubmatch(levelID, -1)
	segs := make([]string, 0, len(matches))
	for _, m := range matches {
		segs = append(segs, "HN"+m[1]+"#"+m[2])
	}
	hn2 := -1
	for i, s := range segs {
		if strings.HasPrefix(s, "HN2#") {
			hn2 = i
			break
		}
	}
	if hn2 < 0 {
		return "", "", fmt.Errorf("level_id has no HN2 (company) segment: %q", levelID)
	}
	path := segs[hn2:]
	return path[0], strings.Join(path, "|"), nil
}

func granOf(resolution string) string {
	if resolution == "daily" {
		return "d"
	}
	return "h"
}

func parseISO(s string) (time.Time, error) {
	t, err := time.Parse(time.RFC3339, strings.TrimSpace(s))
	if err != nil {
		return time.Time{}, err
	}
	return t.UTC(), nil
}

// bucketLabel: UTC bucket label — hour "YYYY-MM-DDThh" | day "YYYY-MM-DD".
func bucketLabel(iso, gran string) (string, error) {
	t, err := parseISO(iso)
	if err != nil {
		return "", err
	}
	if gran == "h" {
		return t.Format("2006-01-02T15"), nil
	}
	return t.Format("2006-01-02"), nil
}

// bucketToISO: bucket label -> ISO-8601 UTC instant (the bucket's start).
func bucketToISO(bucket, gran string) string {
	layout := "2006-01-02"
	if gran == "h" {
		layout = "2006-01-02T15"
	}
	t, err := time.Parse(layout, bucket)
	if err != nil {
		return bucket
	}
	return t.UTC().Format("2006-01-02T15:04:05") + "Z"
}

// parseSK: "<node_path>#<purpose>#<gran>#<bucket>" -> parts (node_path may itself contain '#').
// Mirrors Python's sk.rsplit("#", 3).
func parseSK(sk string) (nodePath, purpose, gran, bucket string) {
	cut := make([]int, 0, 3)
	for i := len(sk) - 1; i >= 0 && len(cut) < 3; i-- {
		if sk[i] == '#' {
			cut = append(cut, i)
		}
	}
	if len(cut) < 3 {
		return sk, "", "", ""
	}
	b, g, p := cut[0], cut[1], cut[2]
	return sk[:p], sk[p+1 : g], sk[g+1 : b], sk[b+1:]
}

type aggItem struct {
	SK    string  `dynamodbav:"sk"`
	Sum   float64 `dynamodbav:"sum"`
	Count int     `dynamodbav:"count"`
	Unit  string  `dynamodbav:"unit"`
}

type row struct {
	LevelID          string  `json:"level_id"`
	Purpose          string  `json:"purpose"`
	Unit             string  `json:"unit"`
	Resolution       string  `json:"resolution"`
	Timestamp        string  `json:"timestamp"`
	Value            float64 `json:"value"`
	ContributorCount int     `json:"contributor_count"`
}

// toRows groups items by purpose (keeping only the requested granularity), time-sorted.
func toRows(items []aggItem, levelID, resolution, gran string) []row {
	type entry struct {
		bucket string
		it     aggItem
	}
	byPurpose := map[string][]entry{}
	for _, it := range items {
		_, purpose, g, bucket := parseSK(it.SK)
		if g != gran {
			continue
		}
		byPurpose[purpose] = append(byPurpose[purpose], entry{bucket, it})
	}
	purposes := make([]string, 0, len(byPurpose))
	for p := range byPurpose {
		purposes = append(purposes, p)
	}
	sort.Strings(purposes)

	rows := []row{}
	for _, purpose := range purposes {
		es := byPurpose[purpose]
		sort.SliceStable(es, func(i, j int) bool { return es[i].bucket < es[j].bucket })
		for _, e := range es {
			rows = append(rows, row{
				LevelID:          levelID,
				Purpose:          purpose,
				Unit:             e.it.Unit,
				Resolution:       resolution,
				Timestamp:        bucketToISO(e.bucket, gran),
				Value:            e.it.Sum,
				ContributorCount: e.it.Count,
			})
		}
	}
	return rows
}

// ── DynamoDB query + handler ──

var ddb *dynamodb.Client

func queryNode(ctx context.Context, pk, skPath, gran, startBucket, endBucket, purpose string) ([]aggItem, error) {
	pkEq := expression.Key("pk").Equal(expression.Value(pk))
	var builder expression.Builder
	if purpose != "" {
		// Efficient range query: fix <path>#<purpose>#<gran># and range the trailing bucket.
		prefix := skPath + "#" + purpose + "#" + gran + "#"
		kc := pkEq.And(expression.Key("sk").Between(
			expression.Value(prefix+startBucket), expression.Value(prefix+endBucket)))
		builder = expression.NewBuilder().WithKeyCondition(kc)
	} else {
		// No purpose: all purposes for the node, narrowed to the bucket range.
		kc := pkEq.And(expression.Key("sk").BeginsWith(skPath + "#"))
		filt := expression.Name("bucket").Between(
			expression.Value(startBucket), expression.Value(endBucket))
		builder = expression.NewBuilder().WithKeyCondition(kc).WithFilter(filt)
	}
	expr, err := builder.Build()
	if err != nil {
		return nil, err
	}
	input := &dynamodb.QueryInput{
		TableName:                 aws.String(rollupTable),
		KeyConditionExpression:    expr.KeyCondition(),
		ExpressionAttributeNames:  expr.Names(),
		ExpressionAttributeValues: expr.Values(),
		FilterExpression:          expr.Filter(),
	}
	var out []aggItem
	p := dynamodb.NewQueryPaginator(ddb, input)
	for p.HasMorePages() {
		page, err := p.NextPage(ctx)
		if err != nil {
			return nil, err
		}
		var batch []aggItem
		if err := attributevalue.UnmarshalListOfMaps(page.Items, &batch); err != nil {
			return nil, err
		}
		out = append(out, batch...)
	}
	return out, nil
}

func resp(status int, body any) (events.LambdaFunctionURLResponse, error) {
	// CORS is added by the Function URL config — do NOT set Access-Control-Allow-Origin here
	// too, or the browser sees duplicate headers.
	b, _ := json.Marshal(body)
	return events.LambdaFunctionURLResponse{
		StatusCode: status,
		Headers:    map[string]string{"Content-Type": "application/json"},
		Body:       string(b),
	}, nil
}

func handler(ctx context.Context, req events.LambdaFunctionURLRequest) (events.LambdaFunctionURLResponse, error) {
	qs := req.QueryStringParameters
	levelID := qs["level_id"]
	resolution := qs["resolution"]
	if resolution == "" {
		resolution = "hourly"
	}
	purpose := qs["purpose"]
	start, end := qs["start"], qs["end"]
	if start == "" || end == "" {
		return resp(400, map[string]string{"error": "start and end are required (ISO-8601)"})
	}
	gran := granOf(resolution)

	pk, skPath, err := parseNodeKeys(levelID)
	if err != nil {
		// node above company level (HN0/HN1) — nothing to aggregate at a single partition.
		return resp(200, []row{})
	}
	startBucket, e1 := bucketLabel(start, gran)
	endBucket, e2 := bucketLabel(end, gran)
	if e1 != nil || e2 != nil {
		return resp(400, map[string]string{"error": "start/end must be ISO-8601 timestamps"})
	}
	items, err := queryNode(ctx, pk, skPath, gran, startBucket, endBucket, purpose)
	if err != nil {
		return resp(500, map[string]string{"error": err.Error()})
	}
	return resp(200, toRows(items, levelID, resolution, gran))
}

func main() {
	cfg, err := config.LoadDefaultConfig(context.Background())
	if err != nil {
		panic(err)
	}
	ddb = dynamodb.NewFromConfig(cfg)
	lambda.Start(handler)
}
