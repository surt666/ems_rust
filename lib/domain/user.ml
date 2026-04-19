type t = {
  id : User_id.t;
  name : string;
  cognito_group : Cognito_group.t;
  language : Language.t;
  currency : Currency.t;
  created : Ptime.t;
}

let make ~email ~name ~cognito_group ?(language = Language.default)
    ?(currency = Currency.default) ~created () =
  {
    id = User_id.of_email email;
    name;
    cognito_group;
    language;
    currency;
    created;
  }
