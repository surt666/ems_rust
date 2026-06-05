type t = { email : string }

let of_email email = { email }
let email t = t.email
let to_string t = "U#" ^ t.email

let of_string s =
  if String.starts_with ~prefix:"U#" s then
    let e = String.sub s 2 (String.length s - 2) in
    if e = "" then Error "empty email after U#" else Ok { email = e }
  else Error "user_id must start with U#"
