open Base
open Ocaml_lambda_test

let uuid s = Stdlib.Option.get (Uuidm.of_string s)

let mk_schema () : Schema.t =
  Schema.{
    version = 1;
    edges = [
      (Level.Hn2, [ (Level.Hn3, [ { label = "property"; min = None; max = None } ]) ]);
      (Level.Hn3, [ (Level.Hn4, [ { label = "building"; min = None; max = None } ]) ]);
    ];
    metadata = [];
    sensors = [];
  }

let seed () =
  let st = Memory.empty () in
  let c2 = Node_id.make Level.Hn2 (uuid "4b6a6f20-0000-0000-0000-00000000bbbb") in
  let n2 =
    Node.make ~uuid:(Node_id.uuid c2) ~level:Level.Hn2 ~name:"Acme"
      ~parent:Node_id.root ~parent_path:(Node_id.to_string Node_id.root) ~created:Ptime.epoch
      ~metadata:(`Assoc []) ~schema:(Some (mk_schema ()))
  in
  Memory.run st (fun () -> Effects.put_node n2);
  (st, c2)

let name_generator =
  Base_quickcheck.Generator.string_non_empty_of Base_quickcheck.Generator.char_print

let add_is_retrievable () =
  Base_quickcheck.Test.run_exn
    (module struct
      type t = string [@@deriving sexp_of]
      let quickcheck_generator = name_generator
      let quickcheck_shrinker = Base_quickcheck.Shrinker.string
    end)
    ~f:(fun name ->
      let st, c2 = seed () in
      Memory.run st (fun () ->
        match
          Hierarchy.add_node
            ~parent:c2 ~level:Level.Hn3 ~name ~metadata:(`Assoc []) ()
        with
        | Error _ -> Alcotest.failf "add_node errored on name=%S" name
        | Ok n ->
            (match Hierarchy.get_node n.Node.id with
             | Ok fetched when String.equal fetched.Node.name name -> ()
             | Ok fetched ->
                 Alcotest.failf "name roundtrip differed: %S vs %S"
                   name fetched.Node.name
             | Error _ -> Alcotest.fail "get_node miss after put")))

let tests = [ Alcotest.test_case "add_node → get_node roundtrip" `Quick add_is_retrievable ]
