FROM golang:1.20-alpine AS builder

WORKDIR /src

COPY go.mod ./
RUN go mod download

COPY cmd ./cmd
COPY internal ./internal

RUN CGO_ENABLED=0 GOOS=linux go build \
    -trimpath \
    -ldflags="-s -w" \
    -o /out/testflying-connector \
    ./cmd/testflying-connector

FROM alpine:3.20 AS runtime

RUN apk add --no-cache ca-certificates \
    && adduser -D -H appuser

COPY --from=builder /out/testflying-connector /usr/local/bin/testflying-connector

USER appuser

EXPOSE 8100

CMD ["testflying-connector"]
