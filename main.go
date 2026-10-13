// anend-server is the old anend blog server: writs (posts) in an embedded
// bbolt database, magic link logins printed to the terminal (or emailed),
// and the admin panel for writing markdown.
package main

import (
	"context"
	"embed"
	"errors"
	"flag"
	"fmt"
	"io/fs"
	"log"
	"net"
	"net/http"
	"os"
	"os/signal"
	"strings"
	"syscall"
	"time"

	"github.com/SaulDoesCode/anend/backend"
)

//go:embed assets templates
var embedded embed.FS

func main() {
	var (
		addr       = flag.String("addr", "localhost:8080", "address to listen on")
		dbPath     = flag.String("db", "data/anend.db", "the database file")
		baseURL    = flag.String("url", "", "the site's public url, used in login links (default http://<addr>)")
		appName    = flag.String("name", "anend", "the site's name, used in login emails")
		adminEmail = flag.String("admin", "", "make the user with this email an admin (creating them if needed) and print a login link for them")
		adminName  = flag.String("admin-name", "", "username for a new -admin user (default: from the email)")
		importPath = flag.String("import", "", "import writs from an arangoexport json dump, then exit")
		dev        = flag.Bool("dev", false, "serve assets/ and templates/ from disk and log every request")
		noLimit    = flag.Bool("no-ratelimit", false, "don't rate limit login requests")
		tlsCert    = flag.String("tls-cert", "", "TLS certificate file (with -tls-key, serve https)")
		tlsKey     = flag.String("tls-key", "", "TLS key file")
		smtpAddr   = flag.String("smtp", "", "SMTP server host:port to email login links through (default: print them)")
		smtpUser   = flag.String("smtp-user", "", "SMTP username (the password comes from $ANEND_SMTP_PASSWORD)")
		smtpFrom   = flag.String("smtp-from", "", "address login emails come from")
	)
	flag.Parse()
	log.SetOutput(os.Stdout)

	store, err := backend.Open(*dbPath)
	if err != nil {
		log.Fatal(err)
	}
	defer store.Close()

	if *importPath != "" {
		f, err := os.Open(*importPath)
		if err != nil {
			log.Fatal(err)
		}
		n, err := store.ImportWrits(f)
		f.Close()
		if err != nil {
			log.Fatalf("import %s: %v", *importPath, err)
		}
		fmt.Printf("imported %d writs from %s into %s\n", n, *importPath, store.Path())
		return
	}

	assets, templates := sub("assets"), sub("templates")
	if *dev {
		assets, templates = os.DirFS("assets"), os.DirFS("templates")
	}

	url := *baseURL
	if url == "" {
		scheme := "http"
		if *tlsCert != "" {
			scheme = "https"
		}
		host := *addr
		if strings.HasPrefix(host, ":") {
			host = "localhost" + host
		}
		url = scheme + "://" + host
	}

	conf := backend.Config{
		AppName:     *appName,
		BaseURL:     url,
		Dev:         *dev,
		NoRateLimit: *noLimit,
	}
	if *smtpAddr != "" {
		if _, _, err := net.SplitHostPort(*smtpAddr); err != nil {
			log.Fatalf("-smtp wants host:port: %v", err)
		}
		conf.Mailer = &backend.Mailer{
			Addr:     *smtpAddr,
			Username: *smtpUser,
			Password: os.Getenv("ANEND_SMTP_PASSWORD"),
			From:     *smtpFrom,
			FromName: *appName,
		}
		if !conf.Mailer.Configured() {
			log.Fatal("-smtp needs -smtp-from too")
		}
	}

	srv, err := backend.NewServer(store, conf, assets, templates)
	if err != nil {
		log.Fatal(err)
	}

	if *adminEmail != "" {
		u, err := store.MakeAdmin(*adminEmail, *adminName)
		if err != nil {
			log.Fatalf("-admin %s: %v", *adminEmail, err)
		}
		log.Printf("%s <%s> is an admin", u.Username, u.Email)
		if _, err := srv.LoginLink(&u); err != nil {
			log.Fatal(err)
		}
	}

	httpServer := &http.Server{
		Addr:              *addr,
		Handler:           srv.Handler(),
		ReadHeaderTimeout: 10 * time.Second,
	}

	go func() {
		for range time.Tick(time.Hour) {
			store.Prune()
		}
	}()

	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer stop()
	go func() {
		<-ctx.Done()
		shutdown, cancel := context.WithTimeout(context.Background(), 5*time.Second)
		defer cancel()
		httpServer.Shutdown(shutdown)
	}()

	login := "login links are printed here"
	if conf.Mailer != nil {
		login = "login links are emailed through " + *smtpAddr
	}
	log.Printf("%s: %d writs in %s; %s", *appName, store.CountWrits(), store.Path(), login)
	log.Printf("serving %s (listening on %s)", url, *addr)

	if *tlsCert != "" {
		err = httpServer.ListenAndServeTLS(*tlsCert, *tlsKey)
	} else {
		err = httpServer.ListenAndServe()
	}
	if err != nil && !errors.Is(err, http.ErrServerClosed) {
		log.Fatal(err)
	}
}

func sub(dir string) fs.FS {
	f, err := fs.Sub(embedded, dir)
	if err != nil {
		panic(err)
	}
	return f
}
