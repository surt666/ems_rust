FROM ocaml/opam:alpine-ocaml-5.4 AS build

USER opam
WORKDIR /home/opam/app

# Install system deps for static linking
RUN sudo apk add --no-cache gmp-dev libev-dev linux-headers openssl-dev openssl-libs-static zlib-dev zlib-static

# Pin lambda-runtime packages (not on public opam repo)
RUN opam pin add -n lambda-runtime git+https://github.com/surt666/ocaml-lambda-runtime.git \
 && opam pin add -n lambda-runtime-api-gateway git+https://github.com/surt666/ocaml-lambda-runtime.git

# Patch smaws (preview): strip the unconditional `Fmt.pr "Response body: %s"` in the AwsJson
# protocol, which dumps every DynamoDB response to stdout -> CloudWatch on every call (log
# bloat + per-call latency, esp. for large subtree responses). Fetch the monorepo source,
# remove the debug print, and pin both packages so opam builds the patched copy.
RUN opam source smaws-lib.0.1~preview1 --dir=/home/opam/smaws \
 && find /home/opam/smaws -path '*protocols_gen*' -name '*.ml' \
      -exec sed -i '/Fmt\.pr "Response body/d' {} + \
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
