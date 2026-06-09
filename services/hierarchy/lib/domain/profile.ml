(* User-facing access profiles. Several profiles collapse onto the three Cognito
   groups; [to_cognito_group] is that mapping. A profile is an input/UI concept
   only — it is not stored. The resulting [Cognito_group.t] is what lives on the
   user. *)
type t = Sysadm | Developer | Standard | Technician | Reader

let all = [ Developer; Standard; Technician; Reader; Sysadm ]

let to_string = function
  | Sysadm     -> "SysAdm"
  | Developer  -> "Developer"
  | Standard   -> "Standard"
  | Technician -> "Technician"
  | Reader     -> "Reader"

let of_string = function
  | "SysAdm"     -> Ok Sysadm
  | "Developer"  -> Ok Developer
  | "Standard"   -> Ok Standard
  | "Technician" -> Ok Technician
  | "Reader"     -> Ok Reader
  | s            -> Error (Printf.sprintf "bad profile %S" s)

let to_cognito_group = function
  | Sysadm                -> Cognito_group.Admin
  | Developer | Standard  -> Cognito_group.Writer
  | Technician | Reader   -> Cognito_group.Reader
