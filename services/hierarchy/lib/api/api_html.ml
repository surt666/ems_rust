(* HTML fragment responses for HTMX endpoints — mirrors the Rust hierarchy
   service's /hierarchy/query/* routes.

   Built on pure-html (the eDSL half of dream-html, sans the Dream/lwt
   integration). Attribute and text values are escaped automatically; HTMX
   attrs come from [Pure_html.Hx] (rendered as [data-hx-*], which HTMX accepts
   identically to the bare [hx-*] form). *)

open Pure_html
open Pure_html.HTML

(* [data-*] is not a first-class helper in pure-html — its [data_] is the rare
   HTML4 <object data="..."> attribute, not the data-* namespace. *)
let data ~suffix fmt = string_attr ("data-" ^ suffix) fmt

let i18n key = data ~suffix:"i18n" "%s" key

(* Percent-encode unreserved-only — for URL components that contain '#' or
   other reserved chars. We can't use [Hx.get] (= [uri_attr]) for parameterised
   URLs because it round-trips through [Uri.of_string], which mistakes the '#'
   in [HN2#<uuid>] for the URL fragment delimiter and re-encodes the '&'
   separators that follow. So we hand-encode components and emit the URL via a
   plain [string_attr] (which only HTML-escapes; the browser DOM parser
   decodes [&amp;] back to [&] when reading the attribute). *)
let pct s =
  let buf = Buffer.create (String.length s) in
  String.iter (fun c -> match c with
    | 'A'..'Z' | 'a'..'z' | '0'..'9' | '-' | '_' | '.' | '~' ->
        Buffer.add_char buf c
    | c -> Buffer.add_string buf (Printf.sprintf "%%%02X" (Char.code c))) s;
  Buffer.contents buf

let hx_get fmt = string_attr "data-hx-get" fmt

let respond ?(status = 200) (n : node) : string =
  let open Lambda_runtime_api_gateway in
  let headers = [ ("content-type", "text/html; charset=utf-8") ] in
  let response =
    Api_gateway.V2.make_response ~status_code:status ~headers (to_string n)
  in
  Yojson.Safe.to_string (Api_gateway.V2.response_to_json response)

let respond_error ?(status = 400) msg =
  respond ~status (div [ class_ "error" ] [ txt "%s" msg ])

(* Tree styling per HN level. Tenant-specific naming (partner/company/...)
   used to be derived from depth, but the meaning of HN3-HN9 is per-tenant
   and lives in the schema config on the HN2 node — see [Schema.t.edges]
   for the labels. Today the schema does not yet carry icon/bar-class
   metadata, so we keep a level → (icon_href, bar_class) fallback that
   reuses the existing sprite IDs. When the schema gains a visual-config
   field, this should consult [Schema_check.find_for] and fall back to
   this table only for unconfigured levels. *)
let level_visual : Level.t -> string * string = function
  | Hn0 -> ("", "")
  | Hn1 -> ("#icon-partner", "partner")
  | Hn2 -> ("#icon-company", "company")
  | Hn3 -> ("#icon-property", "property")
  | Hn4 -> ("#icon-building", "building")
  | Hn5 -> ("#icon-area", "area")
  | Hn6 -> ("#icon-group", "group")
  | Hn7 | Hn8 | Hn9 -> ("#icon-area", "area")

let option_nodes values =
  List.map (fun v -> option [ value "%s" v ] "%s" v) values

let respond_options values = respond (null (option_nodes values))

let render_profiles () =
  respond_options [ "Developer"; "Standard"; "Technician"; "Reader"; "SysAdm" ]

let render_languages () =
  respond_options [ "danish"; "swedish"; "norwegian"; "english"; "german" ]

let render_currencies () =
  respond_options [ "DKK"; "SEK"; "NOK"; "USD"; "EUR" ]

let render_permissions () =
  respond_options [ "view"; "edit"; "admin" ]

let render_timezones () =
  respond_options
    [ "Europe/Copenhagen"; "Europe/Stockholm"; "Europe/Oslo"; "Europe/Berlin";
      "Europe/London"; "Europe/Paris"; "Europe/Madrid"; "Europe/Rome";
      "Europe/Amsterdam"; "UTC" ]

(* Toggle arrow + icon, used in both the standard and permission-grid
   variants of [list_item]. *)
let tree_toggle ~is_leaf ~bar_class =
  if is_leaf then span [ class_ "tree-toggle-spacer" ] []
  else
    std_tag "svg"
      [ class_ "tree-toggle tree-toggle-%s" bar_class;
        width "20"; height "20";
        string_attr "viewBox" "0 0 16 16";
        SVG.fill "currentColor" ]
      [ std_tag "path" [ SVG.d "M3 1l12 7-12 7z" ] [] ]

let tree_icon ~icon_href =
  if icon_href = "" then null []
  else
    std_tag "svg"
      [ string_attr "aria-hidden" "true";
        string_attr "focusable" "false";
        class_ "tree-icon"; width "16"; height "16" ]
      [ std_tag "use" [ string_attr "href" "%s" icon_href ] [] ]

(* Build one tree node. [id_str] is "HN{n}#{uuid}"; [parent_path] is the
   ancestor path joined with '#' (or "H#root" for top-level children).
   [is_leaf] controls whether we emit a toggle arrow. *)
let list_item ~id_str ~level ~user ~node_name ~parent_path ~is_leaf
              ~with_permissions =
  let current_path =
    match parent_path with
    | Some p -> Printf.sprintf "%s#%s" p id_str
    | None -> Printf.sprintf "H#%s" id_str
  in
  let display_name = if node_name = "root" then "" else node_name in
  let icon_href, bar_class = level_visual level in
  let toggle = tree_toggle ~is_leaf ~bar_class in
  let icon = tree_icon ~icon_href in
  let node_path = Option.value parent_path ~default:"" in
  if with_permissions then
    null
      [ div
          [ class_ "permission-row";
            data ~suffix:"id" "%s" id_str;
            data ~suffix:"path" "%s" current_path;
            style_
              "display: grid; grid-template-columns: auto auto auto auto auto \
               auto 1fr; gap: 0.5rem; align-items: center;" ]
          [ toggle; icon;
            input
              [ type_ "checkbox"; name "data.allowed"; value "%s" id_str;
                class_ "allowed-checkbox" ];
            std_tag "svg"
              [ width "16"; height "16"; style_ "color: var(--success);";
                SVG.fill "none"; SVG.stroke "currentColor";
                string_attr "viewBox" "0 0 24 24" ]
              [ std_tag "path"
                  [ SVG.stroke_linecap `round; SVG.stroke_linejoin `round;
                    SVG.stroke_width "2"; SVG.d "M5 13l4 4L19 7" ]
                  [] ];
            input
              [ type_ "checkbox"; name "data.blocked"; value "%s" id_str;
                class_ "blocked-checkbox" ];
            std_tag "svg"
              [ width "16"; height "16"; style_ "color: var(--danger);";
                SVG.fill "none"; SVG.stroke "currentColor";
                string_attr "viewBox" "0 0 24 24" ]
              [ std_tag "path"
                  [ SVG.stroke_linecap `round; SVG.stroke_linejoin `round;
                    SVG.stroke_width "2"; SVG.d "M6 18L18 6M6 6l12 12" ]
                  [] ];
            span [ style_ "font-weight: 600;" ] [ txt "%s" display_name ] ];
        div
          [ class_ "child-rows"; style_ "display:none;";
            hx_get
              "/hierarchy/query/nodes?id=%s&user=%s&path=%s&permissions=true"
              (pct id_str) (pct user) (pct current_path);
            Hx.request {|{"noHeaders": true}|};
            Hx.target "this"; Hx.swap "innerHTML";
            Hx.trigger "loadChildren once" ]
          [] ]
  else
    li
      [ data ~suffix:"id" "%s" id_str;
        data ~suffix:"path" "%s" current_path ]
      [ div
          [ class_ "icon-wrapper";
            hx_get "/hierarchy/query/nodes?id=%s&user=%s&path=%s"
              (pct id_str) (pct user) (pct current_path);
            Hx.request {|{"noHeaders": true}|};
            Hx.target "next .nested-list";
            Hx.trigger "loadChildren";
            Hx.__
              "on click toggle .tree-toggle-expanded on first .tree-toggle \
               in me then get the next .nested-list then if its @style is \
               'display:none;' then set its @style to '' else if its \
               innerHTML is '' then send loadChildren to me else set its \
               @style to 'display:none;' end end" ]
          [ toggle ];
        icon;
        a
          [ href "#"; class_ "node-name-link";
            data ~suffix:"node-id" "%s" id_str;
            data ~suffix:"node-path" "%s" node_path;
            hx_get "/hierarchy/query/node?id=%s&user=%s" (pct id_str) (pct user);
            Hx.request {|{"noHeaders": true}|};
            Hx.target ".main-area"; Hx.swap "innerHTML";
            Hx.__
              "on click remove .selected from .node-name-link in body then \
               add .selected to me then set sessionStorage.selectedNodeId to \
               my @data-node-id then set sessionStorage.selectedNodePath to \
               my @data-node-path";
            style_ "cursor: pointer; text-decoration: none; color: inherit;" ]
          [ txt "%s" display_name ];
        ul [ class_ "nested-list" ] [] ]

(* /hierarchy/query/nodes — list children of id (or top-level for user).
   Gated by administrates grants: a user only sees children of a node if they
   have an Administrates edge on the node or any of its ancestors. No user or
   no grants → empty list (HTMX treats it as "nothing to show"). *)
let render_nodes ~params =
  let user_s = Option.value ~default:"" (List.assoc_opt "user" params) in
  let id_opt = List.assoc_opt "id" params in
  let path_opt = List.assoc_opt "path" params in
  let with_perms =
    match List.assoc_opt "permissions" params with
    | Some "true" -> true | _ -> false
  in
  let parent =
    match id_opt with
    | Some s when s <> "" -> Node_id.of_string s
    | _ -> Ok Node_id.root
  in
  match parent with
  | Error e -> respond_error e
  | Ok parent_id ->
      (* The frontend passes either "U#<email>" or a bare email from login. *)
      let normalized =
        if String.starts_with ~prefix:"U#" user_s then user_s
        else if user_s = "" then ""
        else "U#" ^ user_s
      in
      let allowed =
        match User_id.of_string normalized with
        | Error _ -> false
        | Ok uid -> Access.has_admin_access ~user_id:uid ~node_id:parent_id
      in
      if not allowed then respond (null [])
      else
        (match Hierarchy.list_child_refs parent_id with
         | Error err ->
             respond_error ~status:(Errors.http_status err) (Errors.message err)
         | Ok refs ->
             let parent_path_for_children =
               match id_opt with
               | Some s when s <> "" -> path_opt
               | _ -> Some "H#root"
             in
             let items =
               List.map (fun (id, name) ->
                 let lvl = Node_id.level id in
                 (* Until schema-driven leaf detection is wired up, treat
                    HN4 as the deepest displayable level. *)
                 let is_leaf = lvl = Level.Hn4 in
                 list_item
                   ~id_str:(Node_id.to_string id)
                   ~level:lvl ~user:user_s ~node_name:name
                   ~parent_path:parent_path_for_children
                   ~is_leaf ~with_permissions:with_perms) refs
             in
             respond (null items))

(* Format a metadata JSON blob as a simple key/value form. *)
let rec metadata_rows (j : Yojson.Safe.t) : node =
  match j with
  | `Assoc kvs ->
      null
        (List.map (fun (k, v) ->
           match v with
           | `Assoc _ | `List _ ->
               div [ style_ "margin-top: 1rem;" ]
                 [ h3 [ class_ "table-title" ] [ txt "%s" k ];
                   metadata_rows v ]
           | _ ->
               let s =
                 match v with
                 | `String s -> s
                 | `Int i -> string_of_int i
                 | `Intlit s -> s
                 | `Float f -> string_of_float f
                 | `Bool b -> string_of_bool b
                 | `Null -> ""
                 | _ -> Yojson.Safe.to_string v
               in
               div [ class_ "form-row-2col" ]
                 [ label [ class_ "form-label" ] [ txt "%s:" k ];
                   input
                     [ type_ "text"; value "%s" s; readonly;
                       class_ "form-input" ] ]) kvs)
  | `List xs ->
      null
        (List.mapi (fun i v ->
           div [ style_ "margin-top: 0.5rem;" ]
             [ h4 [] [ txt "[%d]" i ]; metadata_rows v ]) xs)
  | other ->
      div [ class_ "form-row-2col" ]
        [ input
            [ type_ "text"; value "%s" (Yojson.Safe.to_string other);
              readonly; class_ "form-input" ] ]

(* The [hx-on::after-request] inline-JS attribute. Pure-html doesn't ship a
   helper for it (it has Hx.on_ but with a single colon prefix), and the
   value contains literal JS so we render it raw. *)
let hx_on_after_request js =
  string_attr ~raw:true "hx-on::after-request" "%s" js

let sensor_dialog ~nid_str =
  dialog
    [ id "add-sensor-dialog";
      Hx.__ "on click if event.target == me then call me.close()" ]
    [ div [ class_ "dialog-header" ]
        [ h2 [ i18n "node.add_sensor_dialog_title" ] [ txt "ADD SENSOR" ];
          button
            [ type_ "button"; class_ "btn-close";
              Hx.__ "on click call #add-sensor-dialog.close()" ]
            [ txt ~raw:true "&times;" ] ];
      div [ class_ "dialog-body" ]
        [ div
            [ id "sensor-form-error"; class_ "login-error";
              style_ "display:none; margin-bottom: 1rem;" ]
            [];
          form
            [ id "add-sensor-form"; class_ "form";
              Hx.post "/hierarchy/command";
              Hx.swap "none";
              hx_on_after_request
                "if(event.detail.elt.id === 'add-sensor-form' && \
                 event.detail.successful) { \
                 document.querySelector('#add-sensor-dialog').close(); \
                 htmx.trigger('#sensor-list', 'load'); } else if \
                 (event.detail.elt.id === 'add-sensor-form') { \
                 document.getElementById('sensor-form-error').textContent = \
                 event.detail.xhr.responseText; \
                 document.getElementById('sensor-form-error').style.display \
                 = 'block'; }" ]
            [ input [ type_ "hidden"; name "action"; value "attach_sensor" ];
              input [ type_ "hidden"; name "data.parent_id"; value "%s" nid_str ];
              div [ class_ "form-row" ]
                [ label [ class_ "form-label" ] [ txt "DAQ Id" ];
                  input
                    [ type_ "text"; name "data.daq_id"; required;
                      class_ "form-input" ];
                  span [ class_ "required" ] [ txt "*" ] ];
              div [ class_ "form-row" ]
                [ label [ class_ "form-label" ] [ txt "Purpose" ];
                  input
                    [ type_ "text"; name "data.purpose"; required;
                      class_ "form-input" ];
                  span [ class_ "required" ] [ txt "*" ] ];
              div [ class_ "form-row" ]
                [ label [ class_ "form-label" ] [ txt "Meter type" ];
                  select
                    [ name "data.meter_type"; required; class_ "form-select" ]
                    [ option [ value "counter" ] "counter";
                      option [ value "gauge" ] "gauge" ];
                  span [ class_ "required" ] [ txt "*" ] ];
              div [ class_ "form-row" ]
                [ label [ class_ "form-label" ] [ txt "Unit" ];
                  input [ type_ "text"; name "data.unit"; class_ "form-input" ] ];
              div [ class_ "form-row" ]
                [ label [ class_ "form-label" ] [ txt "Binning (min)" ];
                  input
                    [ type_ "number"; name "data.binning";
                      string_attr "min" "1"; string_attr "step" "1";
                      class_ "form-input" ] ] ] ];
      div [ class_ "dialog-footer" ]
        [ button
            [ type_ "submit"; string_attr "form" "add-sensor-form";
              class_ "btn-warning"; i18n "common.save" ]
            [ txt "Save" ];
          div [] [];
          button
            [ type_ "button"; class_ "btn-warning";
              Hx.__ "on click call #add-sensor-dialog.close()";
              i18n "common.close" ]
            [ txt "Close" ] ] ]

let sensor_block ~nid_str ~parent_str =
  div [ style_ "margin-top: 2rem;" ]
    [ div
        [ style_
            "display: grid; grid-template-columns: 1fr auto; align-items: \
             center; margin-bottom: 1rem;" ]
        [ h2 [ class_ "section-title"; style_ "margin-bottom: 0;" ]
            [ span [ i18n "node.sensors" ] [ txt "Sensors" ];
              txt " ";
              span
                [ id "loading-indicator"; class_ "htmx-indicator";
                  style_
                    "display: none; font-size: var(--text-sm); color: \
                     var(--accent); margin-left: 8px;";
                  i18n "common.loading" ]
                [ txt {|Loading…|} ] ];
          button
            [ type_ "button"; class_ "btn-primary";
              Hx.__ "on click call #add-sensor-dialog.showModal()";
              i18n "node.add_sensor" ]
            [ txt "Add sensor" ] ];
      sensor_dialog ~nid_str;
      ul
        [ id "sensor-list"; style_ "display: grid; gap: 8px;";
          Hx.get "/hierarchy/query/sensors";
          Hx.vals {|{"nodepath": "%s"}|} parent_str;
          Hx.trigger "load";
          Hx.target "#sensor-list"; Hx.swap "innerHTML";
          Hx.request {|{"noHeaders": true}|};
          Hx.indicator "#loading-indicator" ]
        [ li
            [ style_ "color: var(--text-muted);"; i18n "node.sensors_loading" ]
            [ txt {|Loading sensors…|} ] ] ]

let add_child_block ~parent_id_str =
  null
    [ div [ style_ "margin-top: 2rem;" ]
        [ div
            [ style_
                "display: grid; grid-template-columns: 1fr auto; \
                 align-items: center; margin-bottom: 1rem;" ]
            [ h2
                [ class_ "section-title"; style_ "margin-bottom: 0;";
                  i18n "node.children" ]
                [ txt "Children" ];
              button
                [ type_ "button"; class_ "btn-primary";
                  Hx.__
                    "on click call #add-child-dialog.showModal() then send \
                     refresh to #add-child-body";
                  i18n "node.add_child" ]
                [ txt "Add child" ] ] ];
      dialog
        [ id "add-child-dialog";
          Hx.__ "on click if event.target == me then call me.close()" ]
        [ div [ class_ "dialog-header" ]
            [ h2 [ i18n "node.add_child_dialog_title" ] [ txt "ADD CHILD" ];
              button
                [ type_ "button"; class_ "btn-close";
                  Hx.__ "on click call #add-child-dialog.close()" ]
                [ txt ~raw:true "&times;" ] ];
          div [ class_ "dialog-body" ]
            [ div
                [ id "add-child-body";
                  Hx.get "/hierarchy/query/add_child_form";
                  Hx.vals {|{"parent": "%s"}|} parent_id_str;
                  Hx.trigger "refresh";
                  Hx.request {|{"noHeaders": true}|};
                  Hx.target "#add-child-body"; Hx.swap "innerHTML" ]
                [ em [ i18n "common.loading" ] [ txt {|Loading…|} ] ] ];
          div [ class_ "dialog-footer" ]
            [ button
                [ type_ "submit"; string_attr "form" "add-child-form";
                  class_ "btn-warning"; i18n "common.save" ]
                [ txt "Save" ];
              div [] [];
              button
                [ type_ "button"; class_ "btn-warning";
                  Hx.__ "on click call #add-child-dialog.close()";
                  i18n "common.close" ]
                [ txt "Close" ] ] ] ]

(* /hierarchy/query/node — node detail page *)
let render_node ~params =
  match List.assoc_opt "id" params with
  | None -> respond_error "missing id"
  | Some id_s ->
      (match Node_id.of_string id_s with
       | Error e -> respond_error e
       | Ok nid ->
           (match Hierarchy.get_node nid with
            | Error err ->
                respond_error ~status:(Errors.http_status err) (Errors.message err)
            | Ok n ->
                let nid_str = Node_id.to_string n.Node.id in
                let parent_str =
                  match n.Node.parent with
                  | None -> nid_str
                  | Some p ->
                      Printf.sprintf "%s#%s" (Node_id.to_string p) nid_str
                in
                let metadata_section =
                  match n.Node.metadata with
                  | `Assoc [] | `Null ->
                      p
                        [ style_ "color: var(--text-muted);";
                          i18n "node.no_metadata" ]
                        [ txt "No metadata available" ]
                  | j -> div [ class_ "form" ] [ metadata_rows j ]
                in
                let level = Node_id.level n.Node.id in
                let show_sensors =
                  match Schema_check.find_for n.Node.id with
                  | Ok (_, schema) -> Schema.allows_sensors schema level
                  | Error _ -> false
                in
                let html =
                  div [ class_ "page-container" ]
                    [ div [ class_ "card" ]
                        [ div [ class_ "form" ]
                            [ div [ class_ "form-row-2col" ]
                                [ label [ class_ "form-label" ] [ txt "ID:" ];
                                  input
                                    [ type_ "text"; id "id";
                                      value "%s" nid_str; readonly;
                                      class_ "form-input" ] ];
                              div [ class_ "form-row-2col" ]
                                [ label
                                    [ class_ "form-label"; i18n "common.name" ]
                                    [ txt "Name" ];
                                  input
                                    [ type_ "text"; id "name";
                                      value "%s" n.Node.name; readonly;
                                      class_ "form-input" ] ] ];
                          add_child_block ~parent_id_str:nid_str;
                          div
                            [ style_
                                "margin-top: 2rem; padding-top: 1.5rem; \
                                 border-top: 1px solid var(--border-medium);" ]
                            [ h2
                                [ class_ "section-title"; i18n "node.metadata" ]
                                [ txt "Metadata" ];
                              metadata_section;
                              if show_sensors then
                                sensor_block ~nid_str ~parent_str
                              else null [] ] ] ]
                in
                respond html))

(* /hierarchy/query/sensors?nodepath=<path> *)
let render_sensors ~params =
  match List.assoc_opt "nodepath" params with
  | None -> respond_error "missing nodepath"
  | Some nodepath ->
      (* nodepath is the concatenated hierarchy path; the leaf id is the last
         "HN{n}#..." segment. *)
      let last =
        match String.rindex_opt nodepath '#' with
        | Some _ ->
            let rec last_node_id i =
              if i < 0 then nodepath
              else
                match nodepath.[i] with
                | 'H' when i + 1 < String.length nodepath
                           && nodepath.[i + 1] = 'N' ->
                    String.sub nodepath i (String.length nodepath - i)
                | _ -> last_node_id (i - 1)
            in
            last_node_id (String.length nodepath - 1)
        | None -> nodepath
      in
      (match Node_id.of_string last with
       | Error e -> respond_error e
       | Ok nid ->
           (match Sensors.list_active ~parent:nid with
            | Error err ->
                respond_error ~status:(Errors.http_status err) (Errors.message err)
            | Ok [] ->
                respond
                  (li
                     [ style_ "color: var(--text-muted);";
                       i18n "node.no_sensors" ]
                     [ txt "No sensors found" ])
            | Ok ss ->
                let items =
                  List.map (fun (s : Sensor.t) ->
                    li [ class_ "sensor-item" ]
                      [ txt "%s (%s)" s.Sensor.daq_id s.Sensor.purpose ]) ss
                in
                respond (null items)))

(* /hierarchy/query/users — <tr> rows *)
let render_users () =
  match Users.list () with
  | Error err ->
      respond_error ~status:(Errors.http_status err) (Errors.message err)
  | Ok [] ->
      respond
        (tr []
           [ td
               [ string_attr "colspan" "6";
                 style_
                   "text-align: center; color: var(--text-muted); padding: \
                    1rem;" ]
               [ txt "No users found" ] ])
  | Ok us ->
      let rows =
        List.map (fun (u : User.t) ->
          let email = User_id.email u.User.id in
          tr []
            [ td [] [ txt "%s" u.User.name ];
              td [] [ txt "%s" email ];
              td [] [ txt "%s" (Cognito_group.to_string u.User.cognito_group) ];
              td [] [ txt "%s" (Language.to_string u.User.language) ];
              td [] [ txt "%s" (Currency.to_string u.User.currency) ];
              td []
                [ button
                    [ class_ "btn-danger btn-sm";
                      string_attr "onclick"
                        "window.dispatchEvent(new \
                         CustomEvent('delete-user', { detail: { email: \
                         '%s' } }))" email ]
                    [ txt "Slet" ] ] ]) us
      in
      respond (null rows)

(* Form input for one schema-defined metadata field. *)
let field_input fname (spec : Metadata.field_spec) =
  let req_attr = if spec.required then required else null_ in
  let req_mark = if spec.required then span [ class_ "required" ] [ txt "*" ]
                 else null [] in
  let common =
    [ name "data.metadata.%s" fname; class_ "form-input"; req_attr ]
  in
  let input_node =
    match spec.typ with
    | Metadata.String _ -> input (type_ "text" :: common)
    | Metadata.Number _ ->
        input (type_ "number" :: string_attr "step" "any" :: common)
    | Metadata.Integer _ ->
        input (type_ "number" :: string_attr "step" "1" :: common)
    | Metadata.Boolean ->
        select
          [ name "data.metadata.%s" fname; class_ "form-select"; req_attr ]
          [ option [ value "true" ] "true";
            option [ value "false" ] "false" ]
    | Metadata.Timestamp ->
        input (type_ "datetime-local" :: common)
    | Metadata.Enum { one_of } ->
        select
          [ name "data.metadata.%s" fname; class_ "form-select"; req_attr ]
          (List.map (fun v -> option [ value "%s" v ] "%s" v) one_of)
  in
  div [ class_ "form-row" ]
    [ label [ class_ "form-label" ] [ txt "%s" fname ]; input_node; req_mark ]

(* /hierarchy/query/add_child_form?parent=<id> — modal dialog body with a
   form for creating a child node under [parent]. If the schema allows more
   than one child level under the parent, a level <select> is emitted (with
   an HTMX hx-get that re-fetches the form for the newly chosen level). The
   metadata fields for the selected (or first allowed) level are rendered
   inline. Post target is /hierarchy/command action=add_node. *)
let render_add_child_form ~params =
  match List.assoc_opt "parent" params with
  | None -> respond_error "missing parent"
  | Some parent_s ->
      (match Node_id.of_string parent_s with
       | Error e -> respond_error e
       | Ok parent_id ->
           (match Hierarchy.get_node parent_id with
            | Error err ->
                respond_error ~status:(Errors.http_status err) (Errors.message err)
            | Ok parent_node ->
                let parent_level = Node_id.level parent_node.Node.id in
                let schema_opt =
                  if parent_level = Level.Hn0 || parent_level = Level.Hn1 then
                    (* HN0 -> HN1 and HN1 -> HN2 are hardcoded; no schema yet. *)
                    None
                  else
                    match Schema_check.find_for parent_id with
                    | Ok (_, s) -> Some s
                    | Error _ -> None
                in
                let allowed : (Level.t * Schema.edge_spec list) list =
                  match parent_level with
                  | Level.Hn0 ->
                      [ Level.Hn1,
                        [ { Schema.label = "partner"; min = None; max = None } ] ]
                  | Level.Hn1 ->
                      [ Level.Hn2,
                        [ { Schema.label = "company"; min = None; max = None } ] ]
                  | _ ->
                      (match schema_opt with
                       | Some sch -> Schema.allowed_children sch parent_level
                       | None -> [])
                in
                let requested_level =
                  match List.assoc_opt "level" params with
                  | Some s ->
                      (match Level.of_string s with
                       | Ok l -> Some l | Error _ -> None)
                  | None -> None
                in
                let chosen_level =
                  match requested_level, allowed with
                  | Some l, xs when List.mem_assoc l xs -> Some l
                  | _, (l, _) :: _ -> Some l
                  | _, [] -> None
                in
                match chosen_level with
                | None ->
                    respond
                      (div [ class_ "error" ]
                         [ txt
                             "This node type cannot have children according \
                              to its schema." ])
                | Some level ->
                    let labels =
                      match List.assoc_opt level allowed with
                      | Some specs -> List.map (fun s -> s.Schema.label) specs
                      | None -> []
                    in
                    let metadata_fields =
                      match schema_opt with
                      | Some sch -> Schema.metadata_for sch level
                      | None -> []
                    in
                    let level_options =
                      List.map (fun (l, _) ->
                        let attrs =
                          if l = level then
                            [ value "%s" (Level.to_string l);
                              attr "selected" ]
                          else
                            [ value "%s" (Level.to_string l) ]
                        in
                        option attrs "%s" (Level.to_string l)) allowed
                    in
                    let level_selector =
                      if List.length allowed <= 1 then
                        div [ class_ "form-row" ]
                          [ label
                              [ class_ "form-label"; i18n "common.type" ]
                              [ txt "Type" ];
                            select
                              [ class_ "form-select"; disabled ]
                              level_options;
                            input
                              [ type_ "hidden"; name "data.level";
                                value "%s" (Level.to_string level) ] ]
                      else
                        div [ class_ "form-row" ]
                          [ label
                              [ class_ "form-label"; i18n "common.type" ]
                              [ txt "Type" ];
                            select
                              [ name "data.level"; class_ "form-select";
                                Hx.get "/hierarchy/query/add_child_form";
                                Hx.trigger "change";
                                Hx.vals {|{"parent": "%s"}|} parent_s;
                                Hx.include_ "this";
                                Hx.target "#add-child-body";
                                Hx.swap "innerHTML" ]
                              level_options ]
                    in
                    let label_selector =
                      match labels with
                      | [] -> null []
                      | [ only ] ->
                          div [ class_ "form-row" ]
                            [ label
                                [ class_ "form-label"; i18n "common.label" ]
                                [ txt "Label" ];
                              select
                                [ class_ "form-select"; disabled ]
                                [ option
                                    [ value "%s" only; attr "selected" ]
                                    "%s" only ];
                              input
                                [ type_ "hidden"; name "data.label";
                                  value "%s" only ] ]
                      | xs ->
                          div [ class_ "form-row" ]
                            [ label
                                [ class_ "form-label"; i18n "common.label" ]
                                [ txt "Label" ];
                              select
                                [ name "data.label"; class_ "form-select" ]
                                (List.map
                                   (fun l -> option [ value "%s" l ] "%s" l)
                                   xs) ]
                    in
                    let metadata_inputs =
                      null
                        (List.map (fun (n, spec) -> field_input n spec)
                           metadata_fields)
                    in
                    let html =
                      null
                        [ div
                            [ id "add-child-error"; class_ "login-error";
                              style_ "display:none; margin-bottom: 1rem;" ]
                            [];
                          form
                            [ id "add-child-form"; class_ "form";
                              Hx.post "/hierarchy/command";
                              Hx.swap "none";
                              hx_on_after_request
                                "if(event.detail.elt.id === \
                                 'add-child-form' && \
                                 event.detail.successful) { \
                                 document.querySelector('#add-child-dialog').close(); \
                                 location.reload(); } else if \
                                 (event.detail.elt.id === \
                                 'add-child-form') { \
                                 document.getElementById('add-child-error').textContent \
                                 = event.detail.xhr.responseText; \
                                 document.getElementById('add-child-error').style.display \
                                 = 'block'; }" ]
                            [ input
                                [ type_ "hidden"; name "action";
                                  value "add_node" ];
                              input
                                [ type_ "hidden"; name "data.parent_id";
                                  value "%s" parent_s ];
                              level_selector;
                              label_selector;
                              div [ class_ "form-row" ]
                                [ label
                                    [ class_ "form-label"; i18n "common.name" ]
                                    [ txt "Name" ];
                                  input
                                    [ type_ "text"; name "data.name";
                                      required; class_ "form-input" ];
                                  span [ class_ "required" ] [ txt "*" ] ];
                              metadata_inputs ] ]
                    in
                    respond html))

let dispatch ~action ~params =
  match action with
  | "profiles"  -> render_profiles ()
  | "languages" -> render_languages ()
  | "currencies" -> render_currencies ()
  | "timezones" -> render_timezones ()
  | "permissions" -> render_permissions ()
  | "nodes"     -> render_nodes ~params
  | "node"      -> render_node  ~params
  | "sensors"   -> render_sensors ~params
  | "users"     -> render_users ()
  | "add_child_form" -> render_add_child_form ~params
  | other       -> respond_error ~status:400
                     (Printf.sprintf "unknown hierarchy action %s" other)
