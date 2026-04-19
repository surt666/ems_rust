type t = DKK | SEK | NOK | USD | EUR

let to_string = function
  | DKK -> "DKK" | SEK -> "SEK" | NOK -> "NOK"
  | USD -> "USD" | EUR -> "EUR"

let of_string = function
  | "DKK" -> Ok DKK
  | "SEK" -> Ok SEK
  | "NOK" -> Ok NOK
  | "USD" -> Ok USD
  | "EUR" -> Ok EUR
  | s -> Error (Printf.sprintf "bad currency %S" s)

let default = DKK
