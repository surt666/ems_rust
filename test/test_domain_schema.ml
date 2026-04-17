open Ocaml_lambda_test

let sample_schema () : Schema.t =
  Schema.{
    version = 1;
    edges = [
      (Level.Hn2, [ (Level.Hn3, [ { label = "property"; min = None; max = None } ]) ]);
      (Level.Hn3, [ (Level.Hn4, [ { label = "building"; min = Some 1; max = None } ]) ]);
      (Level.Hn4, [ (Level.Hn5, [ { label = "area"; min = None; max = None } ]) ]);
    ];
    metadata = [
      (Level.Hn4, [
        ("lat", Metadata.{ typ = Number { min = Some (-90.); max = Some 90. }; required = true });
        ("lng", Metadata.{ typ = Number { min = Some (-180.); max = Some 180. }; required = true });
      ]);
    ];
  }

let self_check_accepts_sample () =
  match Schema.validate (sample_schema ()) with
  | Ok () -> ()
  | Error e -> Alcotest.failf "unexpected validation error: %s" e

let rejects_depth_violation () =
  let bad =
    Schema.{
      (sample_schema ()) with
      edges = [ (Level.Hn4, [ (Level.Hn3, [ { label = "x"; min = None; max = None } ]) ]) ];
    }
  in
  (match Schema.validate bad with
   | Ok () -> Alcotest.fail "expected failure on hn4 -> hn3"
   | Error _ -> ())

let allowed_children_lookup () =
  let s = sample_schema () in
  let kids = Schema.allowed_children s Level.Hn3 in
  Alcotest.(check int) "one child level" 1 (List.length kids);
  let level, specs = List.hd kids in
  Alcotest.(check string) "child is hn4" "hn4" (Level.to_string level);
  Alcotest.(check int) "one label" 1 (List.length specs);
  let spec = List.hd specs in
  Alcotest.(check string) "label" "building" spec.Schema.label

let metadata_for_lookup () =
  let s = sample_schema () in
  let md = Schema.metadata_for s Level.Hn4 in
  Alcotest.(check int) "two fields" 2 (List.length md);
  let md_none = Schema.metadata_for s Level.Hn5 in
  Alcotest.(check int) "no fields at hn5" 0 (List.length md_none)

let tests =
  [
    Alcotest.test_case "self-check accepts sample" `Quick self_check_accepts_sample;
    Alcotest.test_case "rejects depth violation" `Quick rejects_depth_violation;
    Alcotest.test_case "allowed_children lookup" `Quick allowed_children_lookup;
    Alcotest.test_case "metadata_for lookup" `Quick metadata_for_lookup;
  ]
