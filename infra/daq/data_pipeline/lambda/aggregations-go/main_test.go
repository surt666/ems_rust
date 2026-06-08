package main

import (
	"reflect"
	"testing"
)

func TestParseNodeKeysFullPath(t *testing.T) {
	pk, sk, err := parseNodeKeys("H#root#HN1#1#HN2#2#HN3#9#HN4#456")
	if err != nil {
		t.Fatal(err)
	}
	if pk != "HN2#2" || sk != "HN2#2|HN3#9|HN4#456" {
		t.Fatalf("got (%q, %q)", pk, sk)
	}
}

func TestParseNodeKeysCompanyOnly(t *testing.T) {
	pk, sk, err := parseNodeKeys("H#root#HN1#1#HN2#2")
	if err != nil || pk != "HN2#2" || sk != "HN2#2" {
		t.Fatalf("got (%q, %q, %v)", pk, sk, err)
	}
}

func TestParseNodeKeysNoCompanyErrors(t *testing.T) {
	if _, _, err := parseNodeKeys("H#root#HN1#1"); err == nil {
		t.Fatal("expected error for missing HN2")
	}
}

func TestGranOf(t *testing.T) {
	for in, want := range map[string]string{"daily": "d", "hourly": "h", "anything-else": "h"} {
		if got := granOf(in); got != want {
			t.Errorf("granOf(%q)=%q want %q", in, got, want)
		}
	}
}

func TestBucketLabelUTC(t *testing.T) {
	if b, _ := bucketLabel("2026-06-07T08:45:00Z", "h"); b != "2026-06-07T08" {
		t.Errorf("hour: %q", b)
	}
	if b, _ := bucketLabel("2026-06-07T08:45:00+00:00", "d"); b != "2026-06-07" {
		t.Errorf("day: %q", b)
	}
}

func TestBucketToISO(t *testing.T) {
	if s := bucketToISO("2026-06-07T08", "h"); s != "2026-06-07T08:00:00Z" {
		t.Errorf("hour: %q", s)
	}
	if s := bucketToISO("2026-06-07", "d"); s != "2026-06-07T00:00:00Z" {
		t.Errorf("day: %q", s)
	}
}

func TestParseSK(t *testing.T) {
	np, p, g, b := parseSK("HN2#2|HN3#9#Energy#d#2026-06-07")
	if np != "HN2#2|HN3#9" || p != "Energy" || g != "d" || b != "2026-06-07" {
		t.Fatalf("got (%q,%q,%q,%q)", np, p, g, b)
	}
}

func TestToRowsGroupsByPurposeAndKeepsGran(t *testing.T) {
	items := []aggItem{
		{SK: "HN2#2#Energy#d#2026-06-02", Sum: 2.0, Count: 10, Unit: "Wh"},
		{SK: "HN2#2#Energy#d#2026-06-01", Sum: 1.0, Count: 5, Unit: "Wh"},
		{SK: "HN2#2#Water#d#2026-06-01", Sum: 9.0, Count: 3, Unit: "m3"},
		{SK: "HN2#2#Energy#h#2026-06-01T08", Sum: 99.0, Count: 1, Unit: "Wh"}, // wrong gran
	}
	rows := toRows(items, "HN2#2", "daily", "d")
	type tup struct {
		purpose, ts string
		value       float64
		count       int
		unit        string
	}
	got := make([]tup, len(rows))
	for i, r := range rows {
		got[i] = tup{r.Purpose, r.Timestamp, r.Value, r.ContributorCount, r.Unit}
	}
	want := []tup{
		{"Energy", "2026-06-01T00:00:00Z", 1.0, 5, "Wh"},
		{"Energy", "2026-06-02T00:00:00Z", 2.0, 10, "Wh"},
		{"Water", "2026-06-01T00:00:00Z", 9.0, 3, "m3"},
	}
	if !reflect.DeepEqual(got, want) {
		t.Fatalf("got %v\nwant %v", got, want)
	}
}
