#!/usr/bin/env bash
# Seed root via AWS CLI (API cannot create hn0) then drive the live API to
# build partner + 2 companies (each with a different schema) + children, then
# a bulk-seed section that creates N companies / properties / buildings / areas
# / sensors via the API. Re-runnable: wipes the hierarchy table first.
#
# Requirements: aws CLI (with creds), curl, jq

set -euo pipefail

BASE="${BASE:-https://doztw28ic6.execute-api.eu-central-1.amazonaws.com}"
TABLE="${TABLE:-hierarchy_new}"
REGION="${REGION:-eu-central-1}"

# Bulk-seed scale (override to shrink while iterating)
SCALE_COMPANIES="${SCALE_COMPANIES:-10}"
SCALE_PROPERTIES="${SCALE_PROPERTIES:-10}"
SCALE_BUILDINGS="${SCALE_BUILDINGS:-20}"
SCALE_SENSORS="${SCALE_SENSORS:-30}"
PARALLEL="${PARALLEL:-20}"

# API Gateway V2 + AWS_PROXY unwraps the {statusCode, body} envelope; client
# sees the raw body string with the Lambda-chosen status code. We capture the
# body and HTTP status into globals for the caller to inspect.
body=""
status=""
do_curl() {
  local raw; raw=$(curl -sS -w $'\n__STATUS__%{http_code}' "$@")
  body="${raw%$'\n'__STATUS__*}"
  status="${raw##*__STATUS__}"
}
cmd() { do_curl -X POST "${BASE}/command" -H 'content-type: application/json' -d "$1"; }
qry() { local action="$1"; shift; do_curl -G "${BASE}/query/${action}" "$@"; }

section() { printf '\n\033[1;36m==> %s\033[0m\n' "$1"; }

section "Wipe table ${TABLE}"
wipe_dir=$(mktemp -d)
trap 'rm -rf "$wipe_dir"' EXIT
aws dynamodb scan --region "$REGION" --table-name "$TABLE" \
    --projection-expression "pk,sk" --output json \
  | jq -c '.Items[]' \
  | awk 'BEGIN{n=0}
         { if (n==0) printf "{\"'"$TABLE"'\":[";
           else printf ",";
           printf "{\"DeleteRequest\":{\"Key\":%s}}", $0;
           n++;
           if (n==25) { printf "]}\n"; n=0; } }
         END { if (n>0) printf "]}\n" }' \
  | split -l 1 -d -a 5 - "$wipe_dir/b_"
batch_count=$(ls "$wipe_dir" 2>/dev/null | wc -l)
echo "wiping ${batch_count} batches"
if [ "$batch_count" -gt 0 ]; then
  ls "$wipe_dir"/b_* | xargs -P 24 -I{} bash -c '
    f="$1"
    for attempt in 1 2 3 4 5; do
      out=$(aws dynamodb batch-write-item --region "'"$REGION"'" --request-items "file://$f" --return-consumed-capacity NONE --output json 2>&1) || { sleep 0.2; continue; }
      unproc=$(echo "$out" | jq ".UnprocessedItems.\"'"$TABLE"'\" | length // 0" 2>/dev/null || echo 0)
      [ "$unproc" = "0" ] && exit 0
      sleep 0.3
    done
    echo "FAIL $f" >&2
  ' _ {}
fi
echo "wiped"

section "Seed counters at n=10000, live=0"
# Per-level id allocator + cardinality counter. Pre-seed once so id
# allocation starts above the PostgreSQL id range. Idempotent: condition
# fails silently if the row already exists (preserves any drift).
for L in HN1 HN2 HN3 HN4 HN5 HN6 HN7 HN8 HN9 S; do
  aws dynamodb put-item --region "$REGION" --table-name "$TABLE" \
    --condition-expression "attribute_not_exists(pk)" \
    --item '{
      "pk":   {"S":"count#'"$L"'"},
      "sk":   {"S":"count"},
      "type": {"S":"counter"},
      "n":    {"N":"10000"},
      "live": {"N":"0"}
    }' >/dev/null 2>&1 || true
done
echo "counters seeded"

section "Seed root via AWS CLI"
# Root has gsi1sk = its own id (path) but no gsi1pk (HN0/HN1 aren't indexed).
aws dynamodb put-item --region "$REGION" --table-name "$TABLE" --item '{
  "pk":       {"S":"HN0#root"},
  "sk":       {"S":"HN0#root"},
  "type":     {"S":"node"},
  "name":     {"S":"root"},
  "gsi1sk":   {"S":"HN0#root"},
  "created":  {"S":"1970-01-01T00:00:00Z"},
  "metadata": {"M":{}}
}' >/dev/null
echo "root seeded"

section "API: create partner under root"
cmd '{"action":"add_node","parent_id":"HN0#root","level":"hn1","name":"Acme Group"}'
echo "HTTP $status"
echo "$body" | jq .
partner_id=$(echo "$body" | jq -r '.id // empty')
echo "partner_id=${partner_id:-<none>}"
[ -z "$partner_id" ] && { echo "abort: partner not created"; exit 1; }

property_schema='{
  "version": 1,
  "edges": {
    "hn2": {"hn3": {"property": {}, "group": {}}},
    "hn3": {"hn4": {"building": {}}},
    "hn4": {"hn5": {"area": {}}}
  },
  "metadata": {
    "hn4": {
      "lat": {"type":"number","required":true,"min":-90,"max":90},
      "lng": {"type":"number","required":true,"min":-180,"max":180}
    }
  },
  "sensors": ["hn4","hn5"]
}'

chargepoint_schema='{
  "version": 1,
  "edges": {
    "hn2": {"hn3": {"parkinglot": {}}},
    "hn3": {"hn4": {"chargingpool": {}}},
    "hn4": {"hn5": {"charger": {}}},
    "hn5": {"hn6": {"plug": {}}}
  },
  "metadata": {
    "hn5": {
      "power_kw":  {"type":"number","required":true,"min":0},
      "connector": {"type":"enum","required":true,"one_of":["ccs","type2","chademo"]}
    }
  },
  "sensors": []
}'

section "API: create RealEstateCo (property_schema)"
re_body=$(jq -n --arg p "$partner_id" --argjson s "$property_schema" \
  '{action:"add_node", parent_id:$p, level:"hn2", name:"RealEstateCo", schema:$s}')
cmd "$re_body"
echo "HTTP $status"
echo "$body" | jq '{id, name}'
re_id=$(echo "$body" | jq -r '.id // empty')

section "API: create ChargeCo (chargepoint_schema)"
cc_body=$(jq -n --arg p "$partner_id" --argjson s "$chargepoint_schema" \
  '{action:"add_node", parent_id:$p, level:"hn2", name:"ChargeCo", schema:$s}')
cmd "$cc_body"
echo "HTTP $status"
echo "$body" | jq '{id, name}'
cc_id=$(echo "$body" | jq -r '.id // empty')

section "API: ChargeCo + parkinglot (single label, unambiguous)"
cmd "$(jq -n --arg p "$cc_id" '{action:"add_node",parent_id:$p,level:"hn3",name:"Lot A"}')"
echo "HTTP $status"; echo "$body" | jq '{id, name}'

section "API: RealEstateCo child without label -> expected 422 (ambiguous)"
cmd "$(jq -n --arg p "$re_id" '{action:"add_node",parent_id:$p,level:"hn3",name:"Oops"}')"
echo "HTTP $status"; echo "$body" | jq .

section "API: RealEstateCo + property (label=property)"
cmd "$(jq -n --arg p "$re_id" '{action:"add_node",parent_id:$p,level:"hn3",name:"Ostergade",label:"property"}')"
echo "HTTP $status"; echo "$body" | jq '{id, name}'

section "API: RealEstateCo + group (label=group)"
cmd "$(jq -n --arg p "$re_id" '{action:"add_node",parent_id:$p,level:"hn3",name:"Portfolio North",label:"group"}')"
echo "HTTP $status"; echo "$body" | jq '{id, name}'

section "API: RealEstateCo + cross-schema child -> expected 422"
cmd "$(jq -n --arg p "$re_id" '{action:"add_node",parent_id:$p,level:"hn3",name:"Wrong",label:"parkinglot"}')"
echo "HTTP $status"; echo "$body" | jq .

section "Query: list partners under root"
qry list_children --data-urlencode "parent=HN0#root"
echo "HTTP $status"; echo "$body" | jq '.children[] | {id, name}'

section "Query: list companies under partner"
qry list_children --data-urlencode "parent=${partner_id}"
echo "HTTP $status"; echo "$body" | jq '.children[] | {id, name}'

section "Query: list property children of RealEstateCo"
qry list_children --data-urlencode "parent=${re_id}" --data-urlencode "label=property"
echo "HTTP $status"; echo "$body" | jq '.children[] | {id, name}'

#############################################################################
# Bulk seed
#   10 companies * 10 properties * 20 buildings per property
#   every 2nd building gets 2 areas
#   30 sensors per building: no areas -> all on building;
#                            with areas -> 10 building, 10 per area
#   80% counter, 20% gauge  (sensor index 1..24 counter, 25..30 gauge)
#############################################################################

section "Bulk seed: ${SCALE_COMPANIES} companies * ${SCALE_PROPERTIES} props * ${SCALE_BUILDINGS} bldgs"

# Shell helpers that hit the API directly (no shared-global capture because we
# call them from xargs workers). Each helper echoes the new id to stdout on
# success, or fails the script on HTTP error.
# Retry wrapper: POST JSON to /command with up to N attempts on transient 5xx.
# Echos the response body. Caller parses .id.
post_command_with_retry() {
  local payload="$1"
  local max=5 sleep_s=0.2
  local n=0 raw http
  while : ; do
    n=$((n+1))
    raw=$(curl -sS -w $'\n__STATUS__%{http_code}' -X POST "${BASE}/command" \
      -H 'content-type: application/json' -d "$payload" || echo $'\n__STATUS__000')
    http="${raw##*__STATUS__}"
    body="${raw%$'\n'__STATUS__*}"
    case "$http" in
      2??|4??) echo "$body"; return 0 ;;
      *)
        if [ "$n" -ge "$max" ]; then
          echo "$body"
          return 1
        fi
        sleep "$sleep_s"
        sleep_s=$(awk -v s="$sleep_s" 'BEGIN{printf "%.3f", s*2}')
        ;;
    esac
  done
}

api_add_node_id() {
  # args: parent_id level name [label] [metadata_json] [schema_json]
  local parent="$1" level="$2" name="$3"
  local label="${4:-}" meta="${5:-}" sch="${6:-}"
  local payload
  payload=$(jq -n \
    --arg p "$parent" --arg l "$level" --arg n "$name" \
    --arg lbl "$label" \
    --argjson meta "${meta:-null}" \
    --argjson sch "${sch:-null}" \
    '{action:"add_node", parent_id:$p, level:$l, name:$n}
     + (if $lbl == "" then {} else {label:$lbl} end)
     + (if $meta == null then {} else {metadata:$meta} end)
     + (if $sch == null then {} else {schema:$sch} end)')
  local resp
  resp=$(post_command_with_retry "$payload")
  local id
  id=$(echo "$resp" | jq -r '.id // empty')
  if [ -z "$id" ]; then
    echo "add_node failed: parent=$parent level=$level name=$name resp=$resp" >&2
    return 1
  fi
  echo "$id"
}

api_attach_sensor() {
  # args: parent daq purpose meter_type binning_minutes
  local parent="$1" daq="$2" purpose="$3" mt="$4" bin="${5:-15}"
  local payload
  payload=$(jq -n \
    --arg p "$parent" --arg d "$daq" --arg pu "$purpose" --arg mt "$mt" \
    --argjson bn "$bin" \
    '{action:"attach_sensor", parent_id:$p, daq_id:$d, purpose:$pu,
      meter_type:$mt, binning:$bn}')
  local resp
  resp=$(post_command_with_retry "$payload")
  if ! echo "$resp" | jq -e '.id' >/dev/null 2>&1; then
    echo "attach_sensor failed: parent=$parent daq=$daq resp=$resp" >&2
    return 1
  fi
}

export BASE
export -f post_command_with_retry
export -f api_add_node_id
export -f api_attach_sensor

meter_type_for_index() {
  # 1..24 counter, 25..30 gauge (80/20 of 30)
  local i="$1"
  if [ "$i" -le $(( SCALE_SENSORS * 4 / 5 )) ]; then
    echo counter
  else
    echo gauge
  fi
}

build_one_company() {
  local c_idx="$1"
  local co_name
  co_name=$(printf 'SeedCo%02d' "$c_idx")

  local co_id
  co_id=$(api_add_node_id "$partner_id" hn2 "$co_name" "" "" "$property_schema")
  echo "[$co_name] company $co_id"

  local p_idx b_idx
  for p_idx in $(seq 1 "$SCALE_PROPERTIES"); do
    local p_name
    p_name=$(printf 'Prop%02d' "$p_idx")
    local prop_id
    prop_id=$(api_add_node_id "$co_id" hn3 "$p_name" property)
    echo "  [$co_name/$p_name] property $prop_id"

    for b_idx in $(seq 1 "$SCALE_BUILDINGS"); do
      local b_name
      b_name=$(printf 'Bld%02d' "$b_idx")
      # Deterministic-ish lat/lng seeded by indices, keeps values in range.
      local lat lng
      lat=$(awk -v c="$c_idx" -v p="$p_idx" -v b="$b_idx" \
        'BEGIN{srand(c*10000+p*100+b); printf "%.4f", -89 + rand()*178}')
      lng=$(awk -v c="$c_idx" -v p="$p_idx" -v b="$b_idx" \
        'BEGIN{srand(c*20000+p*200+b); printf "%.4f", -179 + rand()*358}')
      local meta
      meta=$(jq -n --argjson la "$lat" --argjson ln "$lng" '{lat:$la, lng:$ln}')
      local bld_id
      bld_id=$(api_add_node_id "$prop_id" hn4 "$b_name" building "$meta")

      local has_areas=0
      local area1_id="" area2_id=""
      if [ $(( b_idx % 2 )) -eq 0 ]; then
        has_areas=1
        area1_id=$(api_add_node_id "$bld_id" hn5 "Area01" area)
        area2_id=$(api_add_node_id "$bld_id" hn5 "Area02" area)
      fi

      # Emit N sensor jobs to a temp file then xargs them in parallel.
      # Binning rotates over a small set of typical aggregation windows.
      local jobs
      jobs=$(mktemp)
      local i parent_for binnings=(5 15 60)
      for i in $(seq 1 "$SCALE_SENSORS"); do
        local mt
        mt=$(meter_type_for_index "$i")
        if [ "$has_areas" -eq 1 ]; then
          if   [ "$i" -le 10 ]; then parent_for="$bld_id"
          elif [ "$i" -le 20 ]; then parent_for="$area1_id"
          else                       parent_for="$area2_id"
          fi
        else
          parent_for="$bld_id"
        fi
        local daq bin
        daq=$(printf 'daq:%s:%02d:%02d:%02d' "$co_name" "$p_idx" "$b_idx" "$i")
        bin="${binnings[$(( (i - 1) % ${#binnings[@]} ))]}"
        printf '%s\t%s\t%s\t%s\t%s\n' "$parent_for" "$daq" "s$i" "$mt" "$bin" >> "$jobs"
      done

      xargs -P "$PARALLEL" -a "$jobs" -I{} bash -c \
        'IFS=$'"'"'\t'"'"' read -r a b c d e <<<"{}"; api_attach_sensor "$a" "$b" "$c" "$d" "$e"'
      rm -f "$jobs"
    done
  done
  echo "[$co_name] done"
}

export -f meter_type_for_index
export -f build_one_company
export partner_id property_schema SCALE_PROPERTIES SCALE_BUILDINGS SCALE_SENSORS PARALLEL

# Run companies in parallel — each company owns its own tree so there is no
# cross-company coordination needed.
pids=()
for c_idx in $(seq 1 "$SCALE_COMPANIES"); do
  ( build_one_company "$c_idx" ) &
  pids+=("$!")
done
rc=0
for p in "${pids[@]}"; do
  wait "$p" || rc=$?
done
[ "$rc" -eq 0 ] || { echo "one or more company builders failed (rc=$rc)"; exit "$rc"; }

section "Bulk seed: counts from DynamoDB"
aws dynamodb scan --region "$REGION" --table-name "$TABLE" \
    --select COUNT --output json | jq '{count: .Count, scanned: .ScannedCount}'

section "Verify item shape (new id scheme + GSI layout)"
# A sensor item's gsi1pk is the HN2 anchor of its tree, gsi1sk is the
# sensor's full path including the sensor id at the tail. The legacy
# attributes (`path`, `parent`, `hierarchy_path`, the parent_of/sensor_of
# verbs) must not appear.
aws dynamodb scan --region "$REGION" --table-name "$TABLE" \
  --filter-expression "#t = :s AND begins_with(sk, :a)" \
  --expression-attribute-names '{"#t":"type"}' \
  --expression-attribute-values '{":s":{"S":"sensor"},":a":{"S":"active#"}}' \
  --max-items 1 --output json \
  | jq '.Items[0] | {
      pk:.pk.S,
      gsi1pk:.gsi1pk.S,
      gsi1sk:.gsi1sk.S,
      binning:(.binning.N | tonumber),
      legacy_path:(has("path")),
      legacy_parent:(has("parent")),
      legacy_hierarchy_path:(has("hierarchy_path"))
    }'

# A node item: gsi1sk is its own path, gsi1pk is HN2 anchor (or absent for
# HN0/HN1).
aws dynamodb scan --region "$REGION" --table-name "$TABLE" \
  --filter-expression "#t = :s AND begins_with(pk, :p)" \
  --expression-attribute-names '{"#t":"type"}' \
  --expression-attribute-values '{":s":{"S":"node"},":p":{"S":"HN3#"}}' \
  --max-items 1 --output json \
  | jq '.Items[0] | {
      pk:.pk.S,
      gsi1pk:.gsi1pk.S,
      gsi1sk:.gsi1sk.S,
      legacy_path:(has("path"))
    }'

# Counters
section "Counter snapshot"
for L in HN1 HN2 HN3 HN4 HN5 S; do
  aws dynamodb get-item --region "$REGION" --table-name "$TABLE" \
    --key '{"pk":{"S":"count#'"$L"'"},"sk":{"S":"count"}}' \
    --output json | jq -r --arg lvl "$L" '.Item // {} | "\($lvl): n=\(.n.N) live=\(.live.N)"'
done

section "Seed admin user + root grant"
cmd '{"action":"create_user","email":"steen666@gmail.com","name":"Steen Larsen","cognito_group":"admin"}'
echo "HTTP $status"; echo "$body" | jq .
cmd '{"action":"grant_administrates","user_id":"U#steen666@gmail.com","node_id":"HN0#root"}'
echo "HTTP $status"; echo "$body" | jq .

section "Done"
