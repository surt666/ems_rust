type t = Reader | Writer | Admin

(* Canonical strings match the real Cognito user-pool group names (capitalised). *)
let to_string = function
  | Reader -> "Reader" | Writer -> "Writer" | Admin -> "Admin"

(* Case-insensitive so both the canonical capitalised names and any legacy
   lowercase rows decode. *)
let of_string s =
  match String.lowercase_ascii s with
  | "reader" -> Ok Reader
  | "writer" -> Ok Writer
  | "admin"  -> Ok Admin
  | _ -> Error (Printf.sprintf "bad cognito group %S" s)
