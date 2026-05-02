type t =
  | Node of { level : Level.t; id : int }
  | Root

let root = Root
let is_root = function Root -> true | Node _ -> false

let make level id = Node { level; id }

let level = function
  | Root -> Level.Hn0
  | Node { level; _ } -> level

let id = function
  | Root -> failwith "Node_id.id: root has no id"
  | Node { id; _ } -> id

let to_string = function
  | Root -> "HN0#root"
  | Node { level; id } ->
      Printf.sprintf "HN%d#%d" (Level.depth level) id

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
                   (match int_of_string_opt rest with
                    | Some id -> Ok (Node { level; id })
                    | None -> Error (Printf.sprintf "bad id in %S" s)))
          | _ -> Error (Printf.sprintf "bad level in %S" s)

let equal a b =
  match a, b with
  | Root, Root -> true
  | Node { level = la; id = ia }, Node { level = lb; id = ib } ->
      Level.depth la = Level.depth lb && ia = ib
  | _ -> false
