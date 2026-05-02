FROM ocaml/opam:alpine-ocaml-5.4 AS build

USER opam
WORKDIR /home/opam/app

# Install system deps for static linking
RUN sudo apk add --no-cache gmp-dev libev-dev linux-headers openssl-dev openssl-libs-static zlib-dev zlib-static

# Pin lambda-runtime packages (not on public opam repo)
RUN opam pin add -n lambda-runtime git+https://github.com/surt666/ocaml-lambda-runtime.git \
 && opam pin add -n lambda-runtime-api-gateway git+https://github.com/surt666/ocaml-lambda-runtime.git

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
