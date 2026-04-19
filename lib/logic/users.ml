let ( let* ) = Result.bind

let not_found id = Errors.Not_found_user id

let create ~email ~name ~cognito_group ?language ?currency () =
  match Effects.get_user (User_id.of_email email) with
  | Some _ ->
      Error (Errors.Conflict
               (Printf.sprintf "user %s already exists" email))
  | None ->
      let created = Effects.now () in
      let u =
        User.make ~email ~name ~cognito_group ?language ?currency
          ~created ()
      in
      Effects.put_user u;
      Ok u

let get id =
  match Effects.get_user id with
  | Some u -> Ok u
  | None -> Error (not_found id)

let update ~id ?name ?cognito_group ?language ?currency () =
  let* u = get id in
  let u' =
    {
      u with
      User.name = Option.value name ~default:u.User.name;
      cognito_group =
        Option.value cognito_group ~default:u.User.cognito_group;
      language = Option.value language ~default:u.User.language;
      currency = Option.value currency ~default:u.User.currency;
    }
  in
  Effects.put_user u';
  Ok u'

let delete id =
  match Effects.get_user id with
  | None -> Error (not_found id)
  | Some _ ->
      Effects.delete_user id;
      Ok id

let list () = Ok (Effects.list_users ())
