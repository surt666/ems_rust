FROM ocaml/opam:alpine-ocaml-5.4 AS build

USER opam
WORKDIR /home/opam/app

# Install system deps for static linking
RUN sudo apk add --no-cache gmp-dev libev-dev linux-headers openssl-dev openssl-libs-static zlib-dev zlib-static

# Pin lambda-runtime packages (not on public opam repo)
RUN opam pin add -n lambda-runtime git+https://github.com/surt666/ocaml-lambda-runtime.git \
 && opam pin add -n lambda-runtime-api-gateway git+https://github.com/surt666/ocaml-lambda-runtime.git

# Patch smaws (preview) before building. Three fixes:
#  3. http/http_eio_connection.ml: the connection pool's `is_valid` only checks whether the LAST
#     request errored — it never detects a socket killed while the Lambda container was frozen
#     (DynamoDB drops the idle TCP). A warm invocation then reuses the dead socket and blocks on a
#     read with no timeout -> 30s function timeout. Track last-use time and treat a connection
#     idle >5s as invalid so the pool replaces it; intra-invocation reuse is preserved.
#  1. AwsJson protocol: strip the unconditional `Fmt.pr "Response body: %s"` that dumps every
#     DynamoDB response to stdout -> CloudWatch on every call (log bloat + per-call latency).
#  2. http/dns.ml: `resolve_address` returns getaddrinfo's FIRST address with no IPv4 preference,
#     no fallback, and no connect timeout. On Lambda (IPv4-only egress) an IPv6 address returned
#     first hangs the TCP connect until the 30s function timeout -> intermittent 503s. Prefer the
#     first IPv4 address instead.
# Fetch the monorepo source, apply both patches, and pin both packages so opam builds the patched copy.
RUN opam source smaws-lib.0.1~preview1 --dir=/home/opam/smaws \
 && find /home/opam/smaws -path '*protocols_gen*' -name '*.ml' \
      -exec sed -i '/Fmt\.pr "Response body/d' {} + \
 && sed -i 's@List.hd_opt addresses@List.hd_opt (List.filter (function `Tcp (ip, _) -> Eio.Net.Ipaddr.fold ~v4:(fun _ -> true) ~v6:(fun _ -> false) ip | _ -> false) addresses)@' \
      /home/opam/smaws/awssdklib/http/dns.ml \
 && grep -q 'Eio.Net.Ipaddr.fold' /home/opam/smaws/awssdklib/http/dns.ml \
 && ! grep -q 'Fmt.pr "Response body' /home/opam/smaws/awssdklib/protocols_gen/AwsJson.ml \
 && sed -i 's@last_error : Http_types.http_failure option ref;@last_error : Http_types.http_failure option ref; now : unit -> Mtime.t; last_used : Mtime.t ref;@' /home/opam/smaws/awssdklib/http/http_eio_connection.ml \
 && sed -i 's@let is_valid connection = Option.is_none !(connection.last_error)@let is_valid connection = Option.is_none !(connection.last_error) \&\& Mtime.Span.to_float_ns (Mtime.span !(connection.last_used) (connection.now ())) < 5e9@' /home/opam/smaws/awssdklib/http/http_eio_connection.ml \
 && sed -i 's@{ http_client = client; last_error = ref None }@(let now () = Eio.Time.Mono.now mono_clock in { http_client = client; last_error = ref None; now; last_used = ref (now ()) })@' /home/opam/smaws/awssdklib/http/http_eio_connection.ml \
 && sed -i 's@let request ~connection ~body ~method_ ~headers ~sw ~on_ready host path =@let request ~connection ~body ~method_ ~headers ~sw ~on_ready host path = let () = connection.last_used := connection.now () in@' /home/opam/smaws/awssdklib/http/http_eio_connection.ml \
 && grep -q 'last_used : Mtime.t ref' /home/opam/smaws/awssdklib/http/http_eio_connection.ml \
 && opam pin add -yn smaws-lib /home/opam/smaws \
 && opam pin add -yn smaws-clients /home/opam/smaws

# Copy opam file first for layer caching
COPY --chown=opam:opam dune-project ocaml-lambda-hierarchy.opam ./

# Install OCaml dependencies
RUN opam install --deps-only -y . 2>&1

# Copy source
COPY --chown=opam:opam . .

# Build binary. -no-pie prevents producing a static-pie ELF on x86_64
# musl, which segfaults on Lambda al2023 init. Plain static is fine.
ENV OCAMLPARAM=_,ccopt=-static,ccopt=-no-pie,cclib=-static,cclib=-no-pie
RUN opam exec -- dune build bin/main.exe --force 2>&1

FROM alpine:3.20 AS package

RUN apk add --no-cache binutils file

COPY --from=build /home/opam/app/_build/default/bin/main.exe /bootstrap
RUN chmod +x /bootstrap && strip /bootstrap

# Verify linkage
RUN file /bootstrap && ldd /bootstrap 2>&1 || true

FROM scratch AS export
COPY --from=package /bootstrap /bootstrap
