package backend

import (
	"fmt"
	"net/http"

	"github.com/SaulDoesCode/echo-memfile"
	"github.com/labstack/echo"
	"github.com/labstack/echo/middleware"
	"golang.org/x/crypto/acme/autocert"
)

type ctx = echo.Context

var (
	// Server - main echo instance
	Server = echo.New()
	mfi    memfile.MemFileInstance
)

func startServer() {
	// Redirect http traffic to https
	if !DevMode {
		go http.ListenAndServe(HTTP_Port, http.HandlerFunc(func(res http.ResponseWriter, req *http.Request) {
			target := "https://" + req.Host + req.URL.Path
			if len(req.URL.RawQuery) > 0 {
				target += "?" + req.URL.RawQuery
			}
			if DevMode {
				fmt.Printf("\nredirect to: %s \n", target)
				fmt.Println(req.RemoteAddr)
			}
			http.Redirect(res, req, target, http.StatusTemporaryRedirect)
		}))
	}

	if DevMode {
		Server.Logger.Fatal(Server.StartTLS(HTTPS_Port, CertFile, KeyFile))
	} else {
		Server.AutoTLSManager.HostPolicy = autocert.HostWhitelist(TLShost)
		Server.AutoTLSManager.Cache = autocert.DirCache(CertDir)
		Server.Logger.Fatal(Server.StartAutoTLS(HTTPS_Port))
	}
	Server.Use(middleware.Recover())
}
