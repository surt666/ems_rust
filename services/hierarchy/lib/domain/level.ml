type t = Hn0 | Hn1 | Hn2 | Hn3 | Hn4 | Hn5 | Hn6 | Hn7 | Hn8 | Hn9

let depth = function
  | Hn0 -> 0 | Hn1 -> 1 | Hn2 -> 2 | Hn3 -> 3 | Hn4 -> 4
  | Hn5 -> 5 | Hn6 -> 6 | Hn7 -> 7 | Hn8 -> 8 | Hn9 -> 9

let of_depth = function
  | 0 -> Some Hn0 | 1 -> Some Hn1 | 2 -> Some Hn2 | 3 -> Some Hn3
  | 4 -> Some Hn4 | 5 -> Some Hn5 | 6 -> Some Hn6 | 7 -> Some Hn7
  | 8 -> Some Hn8 | 9 -> Some Hn9 | _ -> None

let to_string t = Printf.sprintf "hn%d" (depth t)

let of_string s =
  let n = String.length s in
  if n <> 3 || s.[0] <> 'h' || s.[1] <> 'n' then Error (Printf.sprintf "bad level %S" s)
  else
    match s.[2] with
    | '0' .. '9' as c ->
        let d = Char.code c - Char.code '0' in
        (match of_depth d with
         | Some t -> Ok t
         | None -> Error (Printf.sprintf "bad level %S" s))
    | _ -> Error (Printf.sprintf "bad level %S" s)

let compare_depth a b = Int.compare (depth a) (depth b)
