// The stock HTTP/3 client of the gate of PodUtils/podnet#486. quic-go is an
// independent implementation of HTTP/3; this program sends one GET and
// prints the status and the body. Usage: stock-h3 ADDR [PATH]
package main

import (
	"crypto/tls"
	"fmt"
	"io"
	"net/http"
	"os"
	"runtime/debug"
	"time"

	"github.com/quic-go/quic-go"
	"github.com/quic-go/quic-go/http3"
)

// version is the module version of quic-go in this binary.
func version() string {
	info, ok := debug.ReadBuildInfo()
	if !ok {
		return "unknown"
	}
	for _, dep := range info.Deps {
		if dep.Path == "github.com/quic-go/quic-go" {
			return dep.Version
		}
	}
	return "unknown"
}

func main() {
	if len(os.Args) < 2 {
		fmt.Fprintln(os.Stderr, "usage: stock-h3 ADDR [PATH]")
		os.Exit(64)
	}
	addr := os.Args[1]
	path := "/"
	if len(os.Args) > 2 {
		path = os.Args[2]
	}
	tr := &http3.Transport{
		TLSClientConfig: &tls.Config{InsecureSkipVerify: true, NextProtos: []string{"h3"}},
		QUICConfig:      &quic.Config{HandshakeIdleTimeout: 5 * time.Second, MaxIdleTimeout: 10 * time.Second},
	}
	defer tr.Close()
	client := &http.Client{Transport: tr, Timeout: 10 * time.Second}
	resp, err := client.Get("https://" + addr + path)
	if err != nil {
		fmt.Fprintln(os.Stderr, "GET:", err)
		os.Exit(69)
	}
	defer resp.Body.Close()
	body, err := io.ReadAll(io.LimitReader(resp.Body, 1<<20))
	if err != nil {
		fmt.Fprintln(os.Stderr, "body:", err)
		os.Exit(69)
	}
	fmt.Printf("client=quic-go/http3 version=%s proto=%s status=%d body=%q\n", version(), resp.Proto, resp.StatusCode, string(body))
}
