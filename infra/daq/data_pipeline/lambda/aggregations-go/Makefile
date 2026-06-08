# Graviton (arm64) bootstrap zip for the Go aggregations lambda (compare vs the Python one).
NAME = aggregations-go

build: $(NAME).zip

$(NAME).zip: bootstrap
	zip -q $@ bootstrap

bootstrap: main.go go.mod
	GOOS=linux GOARCH=arm64 CGO_ENABLED=0 go build -tags lambda.norpc -trimpath -ldflags="-s -w" -o bootstrap .

test:
	go test ./...

clean:
	rm -f bootstrap $(NAME).zip

.PHONY: build test clean
