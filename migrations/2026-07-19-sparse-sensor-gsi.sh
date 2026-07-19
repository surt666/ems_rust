#!/usr/bin/env bash
# One-off migration: strip gsi1pk/gsi1sk from existing `has_sensor` edges and history
# sensor rows in hierarchy_new, so the sensor GSI partition (S#HN2#<company>) becomes
# sparse (active sensors only). Idempotent — REMOVE on an already-stripped row is a no-op.
set -euo pipefail
export AWS_PROFILE=stel-sb
TABLE=hierarchy_new

strip() {  # $1 = human label, $2..= scan args already built by caller via env
  :
}

# Emit "pk<TAB>sk" lines for a scan, following pagination.
scan_keys() {
  local filter="$1" names="$2" values="$3"
  local start=""
  while :; do
    local args=(--table-name "$TABLE" --filter-expression "$filter"
      --expression-attribute-names "$names" --expression-attribute-values "$values"
      --projection-expression "pk, sk" --output json)
    if [ -n "$start" ]; then args+=(--exclusive-start-key "$start"); fi
    local out; out="$(aws dynamodb scan "${args[@]}")"
    echo "$out" | python3 -c "import sys,json; d=json.load(sys.stdin); [print(i['pk']['S']+'\t'+i['sk']['S']) for i in d.get('Items',[])]"
    start="$(echo "$out" | python3 -c "import sys,json; d=json.load(sys.stdin); print(json.dumps(d['LastEvaluatedKey']) if d.get('LastEvaluatedKey') else '')")"
    [ -z "$start" ] && break
  done
}

update_one() {  # pk sk
  local pk="$1" sk="$2"
  aws dynamodb update-item --table-name "$TABLE" \
    --key "{\"pk\":{\"S\":\"$pk\"},\"sk\":{\"S\":\"$sk\"}}" \
    --update-expression "REMOVE gsi1pk, gsi1sk" >/dev/null
  echo "  stripped: $pk | $sk"
}

echo "== has_sensor edges =="
scan_keys \
  "#t = :edge AND begins_with(sk, :hs) AND attribute_exists(gsi1pk)" \
  '{"#t":"type"}' '{":edge":{"S":"edge"},":hs":{"S":"has_sensor#"}}' \
| while IFS=$'\t' read -r pk sk; do [ -n "$pk" ] && update_one "$pk" "$sk"; done

echo "== history sensor rows =="
scan_keys \
  "#t = :s AND attribute_exists(gsi1pk) AND NOT begins_with(sk, :a)" \
  '{"#t":"type"}' '{":s":{"S":"sensor"},":a":{"S":"active#"}}' \
| while IFS=$'\t' read -r pk sk; do [ -n "$pk" ] && update_one "$pk" "$sk"; done

echo "== done =="
