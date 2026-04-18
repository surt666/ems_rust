# ocaml-lambda-test — Build
#
# Targets:
#   make build             Build a deploy-ready zip for Lambda (Docker, static, arm64)
#   make build-local       Build a local-only zip (not static, host arch — for dev)
#   make clean             Remove all build artifacts and zips
#
# Variables:
#   NAME=ocaml-lambda-test        Artifact base name (default: ocaml-lambda-test)
#   PLATFORM=linux/arm64          Docker platform target (default: linux/arm64)
#
# Deploy:
#   aws lambda create-function \
#     --function-name my-fn \
#     --runtime provided.al2023 \
#     --architectures arm64 \
#     --handler bootstrap \
#     --role arn:aws:iam::ACCOUNT:role/lambda-role \
#     --zip-file fileb://ocaml-lambda-test.zip

.PHONY: build build-local clean

NAME ?= ocaml-lambda-test
PLATFORM ?= linux/arm64

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
