type t =
  | Node of { level : Level.t; uuid : Uuidm.t }
  | Root

let root = Root
let is_root = function Root -> true | Node _ -> false

let make level uuid = Node { level; uuid }

let level = function
  | Root -> Level.Hn0
  | Node { level; _ } -> level

let uuid = function
  | Root -> failwith "Node_id.uuid: root has no uuid"
  | Node { uuid; _ } -> uuid

let to_string = function
  | Root -> "HN0#root"
  | Node { level; uuid } ->
      Printf.sprintf "HN%d#%s" (Level.depth level) (Uuidm.to_string uuid)

let of_string s =
  if s = "HN0#root" then Ok Root
  else
    match String.index_opt s '#' with
    | None -> Error (Printf.sprintf "no '#' in %S" s)
    | Some i ->
        let prefix = String.sub s 0 i in
        let rest = String.sub s (i + 1) (String.length s - i - 1) in
        let n = String.length prefix in
        if n <> 3 || prefix.[0] <> 'H' || prefix.[1] <> 'N' then
          Error (Printf.sprintf "bad prefix in %S" s)
        else
          match prefix.[2] with
          | '0' .. '9' as c ->
              let d = Char.code c - Char.code '0' in
              (match Level.of_depth d with
               | None -> Error (Printf.sprintf "bad level in %S" s)
               | Some level ->
                   (match Uuidm.of_string rest with
                    | None -> Error (Printf.sprintf "bad uuid in %S" s)
                    | Some uuid -> Ok (Node { level; uuid })))
          | _ -> Error (Printf.sprintf "bad level in %S" s)

let equal a b =
  match a, b with
  | Root, Root -> true
  | Node { level = la; uuid = ua }, Node { level = lb; uuid = ub } ->
      Level.depth la = Level.depth lb && Uuidm.equal ua ub
  | _ -> false
