#!/usr/bin/env bash
# Rename the sensor attributes that Task 1/2 renamed in code, on every stored
# sensor row (active AND history, so history decodes too):
#
#   purpose    -> energy_type
#   meter_type -> reading_kind
#   formula    -> dropped
#
# Dev system, D9: rename outright, no read-fallback — which means the stored rows
# have to move too. Idempotent: rows already migrated are skipped.
set -uo pipefail
PROFILE="${PROFILE:-stel-sb}"
TABLE="${TABLE:-hierarchy_new}"
DRY="${DRY:-0}"

scan() {
  aws dynamodb scan --profile "$PROFILE" --table-name "$TABLE" \
    --filter-expression '#t = :s AND attribute_exists(#p)' \
    --expression-attribute-names '{"#t":"type","#p":"purpose"}' \
    --expression-attribute-values '{":s":{"S":"sensor"}}' \
    --projection-expression 'pk,sk,#p,meter_type' \
    --expression-attribute-names '{"#t":"type","#p":"purpose"}' \
    --output json
}

TOTAL=0
RAW=$(aws dynamodb scan --profile "$PROFILE" --table-name "$TABLE" \
  --filter-expression '#t = :s AND attribute_exists(#p)' \
  --expression-attribute-names '{"#t":"type","#p":"purpose"}' \
  --expression-attribute-values '{":s":{"S":"sensor"}}' \
  --output json)

echo "$RAW" | python3 -c '
import json,sys,subprocess,os
prof=os.environ.get("PROFILE","stel-sb"); table=os.environ.get("TABLE","hierarchy_new")
dry=os.environ.get("DRY","0")=="1"
items=json.load(sys.stdin).get("Items",[])
print(f"rows to migrate: {len(items)}")
for it in items:
    pk,sk = it["pk"]["S"], it["sk"]["S"]
    et = it.get("purpose",{}).get("S")
    rk = it.get("meter_type",{}).get("S")
    sets, removes, vals = [], ["purpose","formula"], {}
    if et is not None:
        sets.append("energy_type = :et"); vals[":et"]={"S":et}
    if rk is not None:
        sets.append("reading_kind = :rk"); vals[":rk"]={"S":rk}
        removes.append("meter_type")
    expr = ("SET " + ", ".join(sets) + " " if sets else "") + "REMOVE " + ", ".join(removes)
    cmd = ["aws","dynamodb","update-item","--profile",prof,"--table-name",table,
           "--key",json.dumps({"pk":{"S":pk},"sk":{"S":sk}}),
           "--update-expression",expr]
    if vals: cmd += ["--expression-attribute-values",json.dumps(vals)]
    if dry:
        print("DRY", pk, sk, expr); continue
    r=subprocess.run(cmd,capture_output=True,text=True)
    if r.returncode: print("FAIL",pk,sk,r.stderr.strip()[:160])
    else: print("ok  ",pk,sk,et,rk)
'
