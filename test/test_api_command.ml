open Ocaml_lambda_test

let uuid s = Uuidm.of_string s |> Option.get

let seed () =
  let st = Memory.empty () in
  let c2 = Node_id.make Level.Hn2 (uuid "4b6a6f20-0000-0000-0000-00000000dddd") in
  let schema : Schema.t =
    Schema.{
      version = 1;
      edges = [
        (Level.Hn2, [ (Level.Hn3, { label = "property"; min = None; max = None }) ]);
      ];
      metadata = [];
    }
  in
  let n2 =
    Node.make ~uuid:(Node_id.uuid c2) ~level:Level.Hn2 ~name:"Acme"
      ~parent:Node_id.root ~created:Ptime.epoch
      ~metadata:(`Assoc []) ~schema:(Some schema)
  in
  Memory.run st (fun () -> Effects.put_node n2);
  (st, c2)

let add_node_happy_path () =
  let st, c2 = seed () in
  let body =
    Printf.sprintf
      {|{"action":"add_node","parent_id":%S,"level":"hn3","name":"P","metadata":{}}|}
      (Node_id.to_string c2)
  in
  let resp = Memory.run st (fun () -> Api_command.dispatch ~body) in
  let status =
    Yojson.Safe.Util.(Yojson.Safe.from_string resp |> member "statusCode" |> to_int)
  in
  Alcotest.(check int) "status 200" 200 status

let delete_node_roundtrip () =
  let st, c2 = seed () in
  Memory.run st (fun () ->
    let add_body =
      Printf.sprintf
        {|{"action":"add_node","parent_id":%S,"level":"hn3","name":"P","metadata":{}}|}
        (Node_id.to_string c2)
    in
    let resp = Api_command.dispatch ~body:add_body in
    let inner =
      Yojson.Safe.Util.(Yojson.Safe.from_string resp |> member "body" |> to_string |> Yojson.Safe.from_string)
    in
    let id = Yojson.Safe.Util.(inner |> member "id" |> to_string) in
    let del = Printf.sprintf {|{"action":"delete_node","id":%S}|} id in
    let resp = Api_command.dispatch ~body:del in
    let status =
      Yojson.Safe.Util.(Yojson.Safe.from_string resp |> member "statusCode" |> to_int)
    in
    Alcotest.(check int) "delete status" 200 status)

let invalid_json_is_400 () =
  let resp = Memory.run (Memory.empty ()) (fun () -> Api_command.dispatch ~body:"not json") in
  let status =
    Yojson.Safe.Util.(Yojson.Safe.from_string resp |> member "statusCode" |> to_int)
  in
  Alcotest.(check int) "400" 400 status

let tests =
  [
    Alcotest.test_case "add_node happy path" `Quick add_node_happy_path;
    Alcotest.test_case "delete roundtrip" `Quick delete_node_roundtrip;
    Alcotest.test_case "invalid json -> 400" `Quick invalid_json_is_400;
  ]
