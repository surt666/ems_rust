# ocaml-lambda-hierarchy — Build
#
# Targets:
#   make build             Build a deploy-ready zip for Lambda (Docker, static, x86_64)
#   make build-local       Build a local-only zip (not static, host arch — for dev)
#   make clean             Remove all build artifacts and zips
#
# Variables:
#   NAME=ocaml-lambda-hierarchy        Artifact base name (default: ocaml-lambda-hierarchy)
#   PLATFORM=linux/amd64          Docker platform target (default: linux/amd64)
#
# Deploy:
#   aws lambda create-function \
#     --function-name my-fn \
#     --runtime provided.al2023 \
#     --architectures x86_64 \
#     --handler bootstrap \
#     --role arn:aws:iam::ACCOUNT:role/lambda-role \
#     --zip-file fileb://ocaml-lambda-hierarchy.zip

.PHONY: build build-local clean

NAME ?= ocaml-lambda-hierarchy
PLATFORM ?= linux/amd64

build: $(NAME).zip

$(NAME).zip: bootstrap
	zip $@ bootstrap
	rm bootstrap

bootstrap:
	docker build \
		--platform $(PLATFORM) \
		--output type=local,dest=. \
		.

# Local build (not static, not cross-compiled — for dev/testing only)
build-local: $(NAME)-local.zip

$(NAME)-local.zip: _build/default/bin/main.exe
	cp $< bootstrap
	zip $@ bootstrap
	rm bootstrap

_build/default/bin/main.exe: FORCE
	dune build bin/main.exe

FORCE:

clean:
	dune clean
	rm -f bootstrap *.zip
