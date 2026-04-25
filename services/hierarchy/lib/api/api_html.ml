(* HTML fragment responses for HTMX endpoints — mirrors the Rust
   hierarchy service's /hierarchy/query/* routes. Plain string formatting,
   no jingoo/lwt. *)

let v2_html ?(status = 200) body =
  let open Lambda_runtime_api_gateway in
  let headers = [ ("content-type", "text/html; charset=utf-8") ] in
  let response = Api_gateway.V2.make_response ~status_code:status ~headers body in
  Yojson.Safe.to_string (Api_gateway.V2.response_to_json response)

let html_error ?(status = 400) msg =
  let escaped = String.concat "" (List.map (function
    | '<' -> "&lt;" | '>' -> "&gt;" | '&' -> "&amp;"
    | '"' -> "&quot;" | c -> String.make 1 c) (List.init (String.length msg) (String.get msg))) in
  v2_html ~status (Printf.sprintf "<div class=\"error\">%s</div>" escaped)

let escape s =
  let buf = Buffer.create (String.length s) in
  String.iter (fun c -> match c with
    | '<' -> Buffer.add_string buf "&lt;"
    | '>' -> Buffer.add_string buf "&gt;"
    | '&' -> Buffer.add_string buf "&amp;"
    | '"' -> Buffer.add_string buf "&quot;"
    | '\'' -> Buffer.add_string buf "&#39;"
    | c -> Buffer.add_char buf c) s;
  Buffer.contents buf

let url_encode s =
  let buf = Buffer.create (String.length s) in
  String.iter (fun c ->
    match c with
    | 'A'..'Z' | 'a'..'z' | '0'..'9' | '-' | '_' | '.' | '~' ->
        Buffer.add_char buf c
    | c -> Buffer.add_string buf (Printf.sprintf "%%%02X" (Char.code c))) s;
  Buffer.contents buf

(* Level-based heuristic mapping — matches the EMS convention. *)
let category_of_level lvl =
  match Level.depth lvl with
  | 0 -> "root"
  | 1 -> "partner"
  | 2 -> "company"
  | 3 -> "property"
  | 4 -> "building"
  | _ -> "area"

let icon_href_of_category = function
  | "root" | "partner" -> "#icon-partner"
  | "company" -> "#icon-company"
  | "property" -> "#icon-property"
  | "building" -> "#icon-building"
  | "area" -> "#icon-area"
  | "group" -> "#icon-group"
  | _ -> ""

let bar_class_of_category = function
  | "root" | "partner" -> "partner"
  | c -> c

let options_block values =
  String.concat "" (List.map (fun v ->
    Printf.sprintf "<option value=\"%s\">%s</option>" (escape v) (escape v)) values)

let render_nodetypes () =
  v2_html (options_block ["partner"; "company"; "property"; "building"; "area"])

let render_profiles () =
  v2_html (options_block ["Developer"; "Standard"; "Technician"; "Reader"; "SysAdm"])

let render_languages () =
  v2_html (options_block ["danish"; "swedish"; "norwegian"; "english"; "german"])

let render_currencies () =
  v2_html (options_block ["DKK"; "SEK"; "NOK"; "USD"; "EUR"])

let render_permissions () =
  v2_html (options_block ["view"; "edit"; "admin"])

let render_timezones () =
  let tzs = [
    "Europe/Copenhagen"; "Europe/Stockholm"; "Europe/Oslo"; "Europe/Berlin";
    "Europe/London"; "Europe/Paris"; "Europe/Madrid"; "Europe/Rome";
    "Europe/Amsterdam"; "UTC"
  ] in
  v2_html (options_block tzs)

(* Build one <li> tree node. `id_str` is "HN{n}#{uuid}"; `parent_path` is
   the ancestor path joined with '#' (or "H#root" for top-level children).
   `is_leaf` controls whether we emit a toggle arrow. *)
let list_item ~id_str ~category ~user ~name ~parent_path ~is_leaf ~with_permissions =
  let current_path =
    match parent_path with
    | Some p -> Printf.sprintf "%s#%s" p id_str
    | None -> Printf.sprintf "H#%s" id_str
  in
  let display_name = if name = "root" then "" else name in
  let encoded_id   = url_encode id_str in
  let encoded_path = url_encode current_path in
  let encoded_user = url_encode user in
  let bar_class = bar_class_of_category category in
  let icon_href = icon_href_of_category category in
  let toggle =
    if is_leaf then
      {|<span class="tree-toggle-spacer"></span>|}
    else
      Printf.sprintf
        {|<svg class="tree-toggle tree-toggle-%s" width="20" height="20" viewBox="0 0 16 16" fill="currentColor"><path d="M3 1l12 7-12 7z"/></svg>|}
        bar_class
  in
  let icon =
    if icon_href = "" then ""
    else
      Printf.sprintf
        {|<svg aria-hidden="true" focusable="false" class="tree-icon" width="16" height="16"><use href="%s"></use></svg>|}
        icon_href
  in
  let node_path = Option.value parent_path ~default:"" in
  if with_permissions then
    (* Permission-grid view: <div class="permission-row"> rather than <li>. *)
    Printf.sprintf
      {|<div class="permission-row" data-id="%s" data-path="%s" style="display: grid; grid-template-columns: auto auto auto auto auto auto 1fr; gap: 0.5rem; align-items: center;">%s%s<input type="checkbox" name="data.allowed" value="%s" class="allowed-checkbox" /><svg width="16" height="16" style="color: var(--success);" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M5 13l4 4L19 7"></path></svg><input type="checkbox" name="data.blocked" value="%s" class="blocked-checkbox" /><svg width="16" height="16" style="color: var(--danger);" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12"></path></svg><span style="font-weight: 600;">%s</span></div><div class="child-rows" style="display:none;" hx-get="/hierarchy/query/nodes?id=%s&user=%s&path=%s&permissions=true" hx-request='{"noHeaders": true}' hx-target="this" hx-swap="innerHTML" hx-trigger="loadChildren once"></div>|}
      (escape id_str) (escape current_path) toggle icon
      (escape id_str) (escape id_str) (escape display_name)
      encoded_id encoded_user encoded_path
  else
    Printf.sprintf
      {|<li data-id="%s" data-path="%s"><div class="icon-wrapper" hx-get="/hierarchy/query/nodes?id=%s&user=%s&path=%s" hx-request='{"noHeaders": true}' hx-target="next .nested-list" hx-trigger="loadChildren" _="on click toggle .tree-toggle-expanded on first .tree-toggle in me then get the next .nested-list then if its @style is 'display:none;' then set its @style to '' else if its innerHTML is '' then send loadChildren to me else set its @style to 'display:none;' end end">%s</div>%s<a href="#" class="node-name-link" data-node-id="%s" data-node-path="%s" hx-get="/hierarchy/query/node?id=%s&user=%s" hx-request='{"noHeaders": true}' hx-target=".main-area" hx-swap="innerHTML" _="on click remove .selected from .node-name-link in body then add .selected to me then set sessionStorage.selectedNodeId to my @data-node-id then set sessionStorage.selectedNodePath to my @data-node-path" style="cursor: pointer; text-decoration: none; color: inherit;">%s</a><ul class="nested-list"></ul></li>|}
      (escape id_str) (escape current_path)
      encoded_id encoded_user encoded_path
      toggle icon
      (escape id_str) (escape node_path)
      encoded_id encoded_user
      (escape display_name)

(* /hierarchy/query/nodes — list children of id (or top-level for user).
   Gated by administrates grants: a user only sees children of a node if they
   have an Administrates edge on the node or any of its ancestors. No user or
   no grants → empty list (not an error — HTMX treats it as "nothing to show"). *)
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
    | Some s when s <> "" ->
        (match Node_id.of_string s with
         | Ok p -> Ok p
         | Error e -> Error e)
    | _ -> Ok Node_id.root
  in
  match parent with
  | Error e -> html_error e
  | Ok parent_id ->
      (* The frontend passes either "U#<email>" or a bare email from login. *)
      let normalized =
        if String.length user_s >= 2 && String.sub user_s 0 2 = "U#" then user_s
        else if user_s = "" then ""
        else "U#" ^ user_s
      in
      let allowed =
        match User_id.of_string normalized with
        | Error _ -> false
        | Ok uid -> Access.has_admin_access ~user_id:uid ~node_id:parent_id
      in
      if not allowed then v2_html ""
      else
        (match Hierarchy.list_child_refs parent_id with
         | Error err ->
             html_error ~status:(Errors.http_status err) (Errors.message err)
         | Ok refs ->
             let parent_path_for_children =
               match id_opt with
               | Some s when s <> "" -> path_opt
               | _ -> Some "H#root"
             in
             let html =
               String.concat "" (List.map (fun (id, name) ->
                 let lvl = Node_id.level id in
                 let cat = category_of_level lvl in
                 let is_leaf = cat = "building" in
                 list_item
                   ~id_str:(Node_id.to_string id)
                   ~category:cat ~user:user_s ~name
                   ~parent_path:parent_path_for_children
                   ~is_leaf ~with_permissions:with_perms) refs)
             in
             v2_html html)

(* Format a metadata JSON blob as a simple key/value form. *)
let rec metadata_rows (json : Yojson.Safe.t) : string =
  match json with
  | `Assoc kvs ->
      String.concat "" (List.map (fun (k, v) ->
        match v with
        | `Assoc _ | `List _ ->
            Printf.sprintf
              {|<div style="margin-top: 1rem;"><h3 class="table-title">%s</h3>%s</div>|}
              (escape k) (metadata_rows v)
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
            Printf.sprintf
              {|<div class="form-row-2col"><label class="form-label">%s:</label><input type="text" value="%s" readonly class="form-input"></div>|}
              (escape k) (escape s)) kvs)
  | `List xs ->
      String.concat "" (List.mapi (fun i v ->
        Printf.sprintf
          {|<div style="margin-top: 0.5rem;"><h4>[%d]</h4>%s</div>|}
          i (metadata_rows v)) xs)
  | _ ->
      Printf.sprintf
        {|<div class="form-row-2col"><input type="text" value="%s" readonly class="form-input"></div>|}
        (escape (Yojson.Safe.to_string json))

(* /hierarchy/query/node — node detail page *)
let render_node ~params =
  match List.assoc_opt "id" params with
  | None -> html_error "missing id"
  | Some id_s ->
      (match Node_id.of_string id_s with
       | Error e -> html_error e
       | Ok nid ->
           (match Hierarchy.get_node nid with
            | Error err -> html_error ~status:(Errors.http_status err) (Errors.message err)
            | Ok n ->
                let parent_str =
                  match n.Node.parent with
                  | None -> Node_id.to_string n.Node.id
                  | Some p -> Printf.sprintf "%s#%s" (Node_id.to_string p) (Node_id.to_string n.Node.id)
                in
                let metadata_html =
                  match n.Node.metadata with
                  | `Assoc [] | `Null -> {|<p style="color: var(--text-muted);">No metadata available</p>|}
                  | j -> Printf.sprintf {|<div class="form">%s</div>|} (metadata_rows j)
                in
                let level = Node_id.level n.Node.id in
                let show_sensors =
                  match Schema_check.find_for n.Node.id with
                  | Ok (_, schema) -> Schema.allows_sensors schema level
                  | Error _ -> false
                in
                let nid_str = Node_id.to_string n.Node.id in
                let sensor_block =
                  if not show_sensors then ""
                  else
                    Printf.sprintf
                      {|<div style="margin-top: 2rem;">
  <div style="display: grid; grid-template-columns: 1fr auto; align-items: center; margin-bottom: 1rem;">
    <h2 class="section-title" style="margin-bottom: 0;">Sensors <span id="loading-indicator" class="htmx-indicator" style="display: none; font-size: var(--text-sm); color: var(--accent); margin-left: 8px;">Loading...</span></h2>
    <button type="button" _="on click call #add-sensor-dialog.showModal()" class="btn-primary">Tilføj sensor</button>
  </div>

  <dialog id="add-sensor-dialog" _="on click if event.target == me then call me.close()">
    <div class="dialog-header">
      <h2>TILFØJ SENSOR</h2>
      <button type="button" _="on click call #add-sensor-dialog.close()" class="btn-close">&times;</button>
    </div>
    <div class="dialog-body">
      <div id="sensor-form-error" class="login-error" style="display:none; margin-bottom: 1rem;"></div>
      <form id="add-sensor-form" class="form"
            hx-post="/hierarchy/command"
            hx-swap="none"
            hx-on::after-request="if(event.detail.elt.id === 'add-sensor-form' && event.detail.successful) { document.querySelector('#add-sensor-dialog').close(); htmx.trigger('#sensor-list', 'load'); } else if(event.detail.elt.id === 'add-sensor-form') { document.getElementById('sensor-form-error').textContent = event.detail.xhr.responseText; document.getElementById('sensor-form-error').style.display = 'block'; }">
        <input type="hidden" name="action" value="attach_sensor" />
        <input type="hidden" name="data.parent_id" value="%s" />
        <div class="form-row"><label class="form-label">DAQ Id</label><input type="text" name="data.daq_id" required class="form-input" /><span class="required">*</span></div>
        <div class="form-row"><label class="form-label">Purpose</label><input type="text" name="data.purpose" required class="form-input" /><span class="required">*</span></div>
        <div class="form-row"><label class="form-label">Meter type</label>
          <select name="data.meter_type" required class="form-select">
            <option value="counter">counter</option>
            <option value="gauge">gauge</option>
          </select>
          <span class="required">*</span>
        </div>
        <div class="form-row"><label class="form-label">Unit</label><input type="text" name="data.unit" class="form-input" /></div>
      </form>
    </div>
    <div class="dialog-footer">
      <button type="submit" form="add-sensor-form" class="btn-warning">Gem</button>
      <div></div>
      <button type="button" _="on click call #add-sensor-dialog.close()" class="btn-warning">Luk</button>
    </div>
  </dialog>

  <ul id="sensor-list" style="display: grid; gap: 8px;" hx-get="/hierarchy/query/sensors" hx-vals='{"nodepath": "%s"}' hx-trigger="load" hx-target="#sensor-list" hx-swap="innerHTML" hx-request='{"noHeaders": true}' hx-indicator="#loading-indicator"><li style="color: var(--text-muted);">Loading sensors...</li></ul>
</div>|}
                      (escape nid_str) (escape parent_str)
                in
                let add_child_block =
                  Printf.sprintf
                    {|<div style="margin-top: 2rem;">
  <div style="display: grid; grid-template-columns: 1fr auto; align-items: center; margin-bottom: 1rem;">
    <h2 class="section-title" style="margin-bottom: 0;">Children</h2>
    <button type="button" class="btn-primary" _="on click call #add-child-dialog.showModal() then send refresh to #add-child-body">Add child</button>
  </div>
</div>
<dialog id="add-child-dialog" _="on click if event.target == me then call me.close()">
  <div class="dialog-header">
    <h2>ADD CHILD</h2>
    <button type="button" _="on click call #add-child-dialog.close()" class="btn-close">&times;</button>
  </div>
  <div class="dialog-body">
    <div id="add-child-body"
         hx-get="/hierarchy/query/add_child_form"
         hx-vals='{"parent": "%s"}'
         hx-trigger="refresh"
         hx-request='{"noHeaders": true}'
         hx-target="#add-child-body"
         hx-swap="innerHTML"><em>Loading…</em></div>
  </div>
  <div class="dialog-footer">
    <button type="submit" form="add-child-form" class="btn-warning">Save</button>
    <div></div>
    <button type="button" _="on click call #add-child-dialog.close()" class="btn-warning">Close</button>
  </div>
</dialog>|}
                    (escape (Node_id.to_string n.Node.id))
                in
                let html =
                  Printf.sprintf
                    {|<div class="page-container">
  <div class="card">
    <div class="form">
      <div class="form-row-2col"><label class="form-label">ID:</label><input type="text" id="id" value="%s" readonly class="form-input"></div>
      <div class="form-row-2col"><label class="form-label">Name:</label><input type="text" id="name" value="%s" readonly class="form-input"></div>
    </div>
    %s
    <div style="margin-top: 2rem; padding-top: 1.5rem; border-top: 1px solid var(--border-medium);">
      <h2 class="section-title">Metadata</h2>
      %s
      %s
    </div>
  </div>
</div>|}
                    (escape (Node_id.to_string n.Node.id))
                    (escape n.Node.name)
                    add_child_block
                    metadata_html
                    sensor_block
                in
                v2_html html))

(* /hierarchy/query/sensors?nodepath=<path> *)
let render_sensors ~params =
  match List.assoc_opt "nodepath" params with
  | None -> html_error "missing nodepath"
  | Some nodepath ->
      (* nodepath is the concatenated hierarchy path; the leaf id is the last segment. *)
      let last =
        match String.rindex_opt nodepath '#' with
        | Some _ ->
            (* strip trailing '#'s then find the HN{n}#... at the end *)
            let rec last_node_id i =
              if i < 0 then nodepath
              else
                match nodepath.[i] with
                | 'H' when i + 1 < String.length nodepath && nodepath.[i + 1] = 'N' ->
                    String.sub nodepath i (String.length nodepath - i)
                | _ -> last_node_id (i - 1)
            in
            last_node_id (String.length nodepath - 1)
        | None -> nodepath
      in
      (match Node_id.of_string last with
       | Error e -> html_error e
       | Ok nid ->
           (match Sensors.list_active ~parent:nid with
            | Error err -> html_error ~status:(Errors.http_status err) (Errors.message err)
            | Ok [] -> v2_html {|<li style="color: var(--text-muted);">No sensors found</li>|}
            | Ok ss ->
                let html =
                  String.concat "" (List.map (fun (s : Sensor.t) ->
                    Printf.sprintf {|<li class="sensor-item">%s (%s)</li>|}
                      (escape s.Sensor.daq_id)
                      (escape s.Sensor.purpose)) ss)
                in
                v2_html html))

(* /hierarchy/query/users — <tr> rows *)
let render_users () =
  match Users.list () with
  | Error err -> html_error ~status:(Errors.http_status err) (Errors.message err)
  | Ok [] ->
      v2_html {|<tr><td colspan="6" style="text-align: center; color: var(--text-muted); padding: 1rem;">No users found</td></tr>|}
  | Ok us ->
      let html =
        String.concat "" (List.map (fun (u : User.t) ->
          let email = User_id.email u.User.id in
          Printf.sprintf
            {|<tr>
  <td>%s</td>
  <td>%s</td>
  <td>%s</td>
  <td>%s</td>
  <td>%s</td>
  <td><button class="btn-danger btn-sm" onclick="window.dispatchEvent(new CustomEvent('delete-user', { detail: { email: '%s' } }))">Slet</button></td>
</tr>|}
            (escape u.User.name)
            (escape email)
            (escape (Cognito_group.to_string u.User.cognito_group))
            (escape (Language.to_string u.User.language))
            (escape (Currency.to_string u.User.currency))
            (escape email)) us)
      in
      v2_html html

(* /hierarchy/query/add_child_form?parent=<id> — modal dialog body with a
   form for creating a child node under [parent]. If the schema allows more
   than one child level under the parent, a level <select> is emitted (with an
   HTMX hx-get that re-fetches the form for the newly chosen level). The
   metadata fields for the selected (or first allowed) level are rendered
   inline. Post target is /hierarchy/command action=add_node. *)
let field_input name (spec : Metadata.field_spec) =
  let required_attr = if spec.required then " required" else "" in
  let required_mark =
    if spec.required then {|<span class="required">*</span>|} else ""
  in
  let input_html =
    match spec.typ with
    | Metadata.String _ ->
        Printf.sprintf
          {|<input type="text" name="data.metadata.%s"%s class="form-input" />|}
          (escape name) required_attr
    | Metadata.Number _ ->
        Printf.sprintf
          {|<input type="number" step="any" name="data.metadata.%s"%s class="form-input" />|}
          (escape name) required_attr
    | Metadata.Integer _ ->
        Printf.sprintf
          {|<input type="number" step="1" name="data.metadata.%s"%s class="form-input" />|}
          (escape name) required_attr
    | Metadata.Boolean ->
        Printf.sprintf
          {|<select name="data.metadata.%s"%s class="form-select"><option value="true">true</option><option value="false">false</option></select>|}
          (escape name) required_attr
    | Metadata.Timestamp ->
        Printf.sprintf
          {|<input type="datetime-local" name="data.metadata.%s"%s class="form-input" />|}
          (escape name) required_attr
    | Metadata.Enum { one_of } ->
        let opts =
          String.concat ""
            (List.map (fun v ->
               Printf.sprintf {|<option value="%s">%s</option>|}
                 (escape v) (escape v)) one_of)
        in
        Printf.sprintf
          {|<select name="data.metadata.%s"%s class="form-select">%s</select>|}
          (escape name) required_attr opts
  in
  Printf.sprintf
    {|<div class="form-row"><label class="form-label">%s</label>%s%s</div>|}
    (escape name) input_html required_mark

let render_add_child_form ~params =
  match List.assoc_opt "parent" params with
  | None -> html_error "missing parent"
  | Some parent_s ->
      (match Node_id.of_string parent_s with
       | Error e -> html_error e
       | Ok parent_id ->
           (match Hierarchy.get_node parent_id with
            | Error err ->
                html_error ~status:(Errors.http_status err) (Errors.message err)
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
                      [ (Level.Hn1, [ { Schema.label = "partner"; min = None; max = None } ]) ]
                  | Level.Hn1 ->
                      [ (Level.Hn2, [ { Schema.label = "company"; min = None; max = None } ]) ]
                  | _ ->
                      (match schema_opt with
                       | Some sch -> Schema.allowed_children sch parent_level
                       | None -> [])
                in
                (* Pick the requested level if any, else first allowed. *)
                let requested_level =
                  match List.assoc_opt "level" params with
                  | Some s ->
                      (match Level.of_string s with Ok l -> Some l | Error _ -> None)
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
                    v2_html
                      {|<div class="error">This node type cannot have children according to its schema.</div>|}
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
                    let level_opts =
                      String.concat ""
                        (List.map (fun (l, _) ->
                           let selected = if l = level then " selected" else "" in
                           Printf.sprintf
                             {|<option value="%s"%s>%s</option>|}
                             (Level.to_string l) selected (Level.to_string l))
                           allowed)
                    in
                    let level_selector =
                      if List.length allowed <= 1 then
                        Printf.sprintf
                          {|<div class="form-row">
  <label class="form-label">Type</label>
  <select class="form-select" disabled>%s</select>
  <input type="hidden" name="data.level" value="%s" />
</div>|}
                          level_opts (Level.to_string level)
                      else
                        Printf.sprintf
                          {|<div class="form-row">
  <label class="form-label">Type</label>
  <select name="data.level" class="form-select"
          hx-get="/hierarchy/query/add_child_form"
          hx-trigger="change"
          hx-vals='{"parent": "%s"}'
          hx-include="this"
          hx-target="#add-child-body" hx-swap="innerHTML">%s</select>
</div>|}
                          (escape parent_s) level_opts
                    in
                    let label_selector =
                      match labels with
                      | [] -> ""
                      | [ only ] ->
                          Printf.sprintf
                            {|<div class="form-row">
  <label class="form-label">Label</label>
  <select class="form-select" disabled><option value="%s" selected>%s</option></select>
  <input type="hidden" name="data.label" value="%s" />
</div>|}
                            (escape only) (escape only) (escape only)
                      | xs ->
                          let opts =
                            String.concat ""
                              (List.map (fun l ->
                                 Printf.sprintf {|<option value="%s">%s</option>|}
                                   (escape l) (escape l)) xs)
                          in
                          Printf.sprintf
                            {|<div class="form-row"><label class="form-label">Label</label><select name="data.label" class="form-select">%s</select></div>|}
                            opts
                    in
                    let metadata_html =
                      String.concat ""
                        (List.map (fun (n, spec) -> field_input n spec) metadata_fields)
                    in
                    let html =
                      Printf.sprintf
                        {|<div id="add-child-error" class="login-error" style="display:none; margin-bottom: 1rem;"></div>
<form id="add-child-form" class="form"
      hx-post="/hierarchy/command"
      hx-swap="none"
      hx-on::after-request="if(event.detail.elt.id === 'add-child-form' && event.detail.successful) { document.querySelector('#add-child-dialog').close(); location.reload(); } else if(event.detail.elt.id === 'add-child-form') { document.getElementById('add-child-error').textContent = event.detail.xhr.responseText; document.getElementById('add-child-error').style.display = 'block'; }">
  <input type="hidden" name="action" value="add_node" />
  <input type="hidden" name="data.parent_id" value="%s" />
  %s
  %s
  <div class="form-row"><label class="form-label">Name</label><input type="text" name="data.name" required class="form-input" /><span class="required">*</span></div>
  %s
</form>|}
                        (escape parent_s)
                        level_selector
                        label_selector
                        metadata_html
                    in
                    v2_html html))

let dispatch ~action ~params =
  match action with
  | "nodetypes" -> render_nodetypes ()
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
  | other       -> html_error ~status:400 (Printf.sprintf "unknown hierarchy action %s" other)
