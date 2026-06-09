NAME = cognito-sync
build: $(NAME).zip
$(NAME).zip: bootstrap
	zip -q $@ bootstrap
bootstrap: main.go go.mod
	GOOS=linux GOARCH=arm64 CGO_ENABLED=0 go build -tags lambda.norpc -trimpath -ldflags="-s -w" -o bootstrap .
clean:
	rm -f bootstrap $(NAME).zip
.PHONY: build clean
