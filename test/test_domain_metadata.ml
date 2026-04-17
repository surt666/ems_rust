open Ocaml_lambda_test

let spec_string ?(required=false) ?min_len ?max_len () =
  Metadata.{ typ = String { min_len; max_len }; required }

let spec_number ?(required=false) ?min ?max () =
  Metadata.{ typ = Number { min; max }; required }

let spec_enum ?(required=false) one_of =
  Metadata.{ typ = Enum { one_of }; required }

let validate_ok () =
  let specs =
    [
      ("lat", spec_number ~required:true ~min:(-90.) ~max:90. ());
      ("lng", spec_number ~required:true ~min:(-180.) ~max:180. ());
      ("name", spec_string ~required:true ~min_len:1 ());
    ]
  in
  let values =
    `Assoc [
      ("lat", `Float 55.68);
      ("lng", `Float 12.57);
      ("name", `String "Building A");
      ("ignored_unknown", `String "ok");
    ]
  in
  match Metadata.validate ~specs values with
  | Ok () -> ()
  | Error errs -> Alcotest.failf "expected Ok, got %d errors" (List.length errs)

let validate_missing_required () =
  let specs = [ ("lat", spec_number ~required:true ()) ] in
  let values = `Assoc [] in
  match Metadata.validate ~specs values with
  | Ok () -> Alcotest.fail "expected Error on missing required"
  | Error [ err ] ->
      Alcotest.(check string) "field" "lat" err.Metadata.path;
      Alcotest.(check bool) "message mentions required" true
        (Astring.String.is_infix ~affix:"required" err.Metadata.message)
  | Error _ -> Alcotest.fail "expected exactly 1 error"

let validate_number_out_of_range () =
  let specs = [ ("lat", spec_number ~required:true ~min:(-90.) ~max:90. ()) ] in
  let values = `Assoc [ ("lat", `Float 200.) ] in
  (match Metadata.validate ~specs values with
   | Ok () -> Alcotest.fail "expected Error on out-of-range"
   | Error _ -> ())

let validate_enum () =
  let specs = [ ("kind", spec_enum ~required:true [ "ccs"; "type2" ]) ] in
  (match Metadata.validate ~specs (`Assoc [ ("kind", `String "chademo") ]) with
   | Ok () -> Alcotest.fail "expected Error on bad enum"
   | Error _ -> ());
  (match Metadata.validate ~specs (`Assoc [ ("kind", `String "ccs") ]) with
   | Ok () -> ()
   | Error _ -> Alcotest.fail "expected Ok on good enum")

let rejects_empty_enum () =
  let spec = Metadata.{ typ = Enum { one_of = [] }; required = false } in
  match Metadata.validate_spec spec with
  | Ok () -> Alcotest.fail "empty enum must be rejected"
  | Error _ -> ()

let tests =
  [
    Alcotest.test_case "validate ok" `Quick validate_ok;
    Alcotest.test_case "missing required" `Quick validate_missing_required;
    Alcotest.test_case "number out of range" `Quick validate_number_out_of_range;
    Alcotest.test_case "enum good/bad" `Quick validate_enum;
    Alcotest.test_case "spec rejects empty enum" `Quick rejects_empty_enum;
  ]
