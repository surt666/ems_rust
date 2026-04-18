type t = Uuidm.t

let make u = u
let uuid t = t
let equal = Uuidm.equal

let to_string t = "S#" ^ Uuidm.to_string t

let of_string s =
  if String.length s < 3 || String.sub s 0 2 <> "S#" then
    Error (Printf.sprintf "missing S# prefix in %S" s)
  else
    let rest = String.sub s 2 (String.length s - 2) in
    match Uuidm.of_string rest with
    | Some u -> Ok u
    | None -> Error (Printf.sprintf "bad uuid in %S" s)
