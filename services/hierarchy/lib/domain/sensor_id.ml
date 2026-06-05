type t = int

let make i = i
let id t = t
let equal = Int.equal

let to_string t = "S#" ^ string_of_int t

let of_string s =
  if not (String.starts_with ~prefix:"S#" s) then
    Error (Printf.sprintf "missing S# prefix in %S" s)
  else
    let rest = String.sub s 2 (String.length s - 2) in
    match int_of_string_opt rest with
    | Some i -> Ok i
    | None -> Error (Printf.sprintf "bad id in %S" s)
