type t = { email : string }

let of_email email = { email }
let email t = t.email
let to_string t = "U#" ^ t.email

let of_string s =
  if String.length s > 2 && String.sub s 0 2 = "U#" then
    let e = String.sub s 2 (String.length s - 2) in
    if e = "" then Error "empty email after U#" else Ok { email = e }
  else Error "user_id must start with U#"
