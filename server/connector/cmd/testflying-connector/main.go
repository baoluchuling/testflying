package main

import (
	"context"
	"log"
	"net/http"
	"os"
	"os/signal"
	"syscall"

	"github.com/baoluchuling/testflying-server/connector/internal/connector"
)

func main() {
	settings, err := connector.LoadSettings()
	if err != nil {
		log.Fatalf("load connector settings: %v", err)
	}
	server, err := connector.NewServer(settings)
	if err != nil {
		log.Fatalf("configure connector: %v", err)
	}

	if settings.CenterURL != "" {
		ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
		defer stop()
		log.Printf(
			"testflying-connector active mode center=%s account=%s mode=%s",
			settings.CenterURL,
			settings.DeveloperAccountID,
			settings.StoreMode,
		)
		if err := connector.RunActiveAgent(ctx, settings, server); err != nil && err != context.Canceled {
			log.Fatal(err)
		}
		return
	}

	log.Printf(
		"testflying-connector listening on %s account=%s mode=%s",
		settings.ListenAddr,
		settings.DeveloperAccountID,
		settings.StoreMode,
	)
	if err := http.ListenAndServe(settings.ListenAddr, server); err != nil {
		log.Fatal(err)
	}
}
