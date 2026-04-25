let table () =
  match Sys.getenv_opt "HIERARCHY_TABLE" with
  | Some t -> t
  | None -> "hierarchy"

let () =
  Eio_main.run @@ fun env ->
  Eio.Switch.run @@ fun sw ->
  let ctx = Smaws_Lib.Context.make ~sw env in
  let cfg = Ocaml_lambda_hierarchy.Dynamo.{ ctx; table = table () } in
  let handler ctx body =
    Ocaml_lambda_hierarchy.Dynamo.run cfg (fun () ->
      Ocaml_lambda_hierarchy.Handler.handler ctx body)
  in
  Lambda_runtime.start handler
