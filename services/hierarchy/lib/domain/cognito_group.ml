type t = Reader | Writer | Admin

let to_string = function
  | Reader -> "reader" | Writer -> "writer" | Admin -> "admin"

let of_string = function
  | "reader" -> Ok Reader
  | "writer" -> Ok Writer
  | "admin"  -> Ok Admin
  | s -> Error (Printf.sprintf "bad cognito group %S" s)

(* capability ordering: admin > writer > reader *)
let rank = function Reader -> 0 | Writer -> 1 | Admin -> 2
