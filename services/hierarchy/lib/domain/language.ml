type t = Danish | Swedish | Norwegian | English | German

let to_string = function
  | Danish -> "danish" | Swedish -> "swedish" | Norwegian -> "norwegian"
  | English -> "english" | German -> "german"

let of_string = function
  | "danish" -> Ok Danish
  | "swedish" -> Ok Swedish
  | "norwegian" -> Ok Norwegian
  | "english" -> Ok English
  | "german" -> Ok German
  | s -> Error (Printf.sprintf "bad language %S" s)

let default = Danish
