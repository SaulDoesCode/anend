package backend

import (
	"fmt"
	"html/template"
	"io/fs"
	"log"
	"net"
	"net/http"
	"path"
	"strings"
	"sync"
	"time"
)

// Config is how the server is set up.
type Config struct {
	AppName     string
	BaseURL     string  // where people reach the site, e.g. http://localhost:8080; used for login links
	Dev         bool    // re-read templates on every request and log every request
	NoRateLimit bool    // don't limit login requests
	Mailer      *Mailer // optional; without it login links go to the terminal
	Log         *log.Logger
}

// Server serves the blog: the static front page and admin panel from assets,
// writ pages from templates, and the json api they use.
type Server struct {
	Store     *Store
	conf      Config
	assets    fs.FS
	templates fs.FS
	tmpl      *template.Template
	limits    *limiter
	log       *log.Logger
}

// NewServer sets up a server over a store, with the assets and templates
// folders as file systems.
func NewServer(store *Store, conf Config, assets, templates fs.FS) (*Server, error) {
	if conf.AppName == "" {
		conf.AppName = "anend"
	}
	conf.BaseURL = strings.TrimRight(conf.BaseURL, "/")
	if conf.Log == nil {
		conf.Log = log.Default()
	}
	s := &Server{
		Store:     store,
		conf:      conf,
		assets:    assets,
		templates: templates,
		limits:    newLimiter(),
		log:       conf.Log,
	}
	t, err := s.parseTemplates()
	if err != nil {
		return nil, err
	}
	s.tmpl = t
	if _, err := fs.Stat(assets, "index.html"); err != nil {
		return nil, fmt.Errorf("the assets have no index.html: %w", err)
	}
	return s, nil
}

func (s *Server) parseTemplates() (*template.Template, error) {
	return template.ParseFS(s.templates, "*.html", "*.txt")
}

func (s *Server) template() *template.Template {
	if s.conf.Dev {
		if t, err := s.parseTemplates(); err == nil {
			return t
		} else {
			s.log.Println("templates:", err)
		}
	}
	return s.tmpl
}

// Handler is the server's routes wrapped in request logging.
func (s *Server) Handler() http.Handler {
	mux := http.NewServeMux()

	mux.HandleFunc("GET /check-username/{username}", s.checkUsername)
	mux.HandleFunc("POST /auth", s.requestLogin)
	mux.HandleFunc("GET /auth/{verifier}", s.login)
	mux.HandleFunc("GET /logout", s.logout)
	mux.HandleFunc("GET /subscribe-toggle", s.authed(s.subscribeToggle))

	mux.HandleFunc("GET /writ/{slug}", s.writPage)
	mux.HandleFunc("GET /like-writ/{slug}", s.authed(s.likeWrit))
	mux.HandleFunc("GET /writs/{page}/{count}", s.listWrits)
	mux.HandleFunc("GET /writs-by-tag/{tag}/{page}/{count}", s.listWrits)

	mux.HandleFunc("GET /admin", s.admin(s.adminPage))
	mux.HandleFunc("POST /writ", s.admin(s.saveWrit))
	mux.HandleFunc("POST /writ-query", s.admin(s.queryWrits))
	mux.HandleFunc("GET /writ-delete/{key}", s.admin(s.deleteWrit))
	mux.HandleFunc("GET /_updateapp", s.admin(func(w http.ResponseWriter, r *http.Request, _ *User) {
		// the old server rebuilt and restarted itself from here; now whatever
		// runs it (a terminal, systemd) does that
		w.Header().Set("Content-Type", "text/html; charset=utf-8")
		w.WriteHeader(http.StatusNotImplemented)
		w.Write([]byte("<h3>the server doesn't restart itself any more, restart it where it runs</h3>"))
	}))

	mux.HandleFunc("GET /", s.static)

	return s.logRequests(mux)
}

// static serves the assets folder, with its 404.html for anything missing.
func (s *Server) static(w http.ResponseWriter, r *http.Request) {
	name := strings.TrimPrefix(path.Clean(r.URL.Path), "/")
	if name == "" {
		name = "index.html"
	}
	st, err := fs.Stat(s.assets, name)
	if err != nil || st.IsDir() {
		s.notFound(w, r)
		return
	}
	if ext := path.Ext(name); ext == ".png" || ext == ".webp" || ext == ".svg" {
		w.Header().Set("Cache-Control", "public, max-age=2592000")
	}
	http.ServeFileFS(w, r, s.assets, name)
}

func (s *Server) notFound(w http.ResponseWriter, r *http.Request) {
	page, err := fs.ReadFile(s.assets, "404.html")
	if err != nil {
		http.NotFound(w, r)
		return
	}
	w.Header().Set("Content-Type", "text/html; charset=utf-8")
	w.WriteHeader(http.StatusNotFound)
	w.Write(page)
}

func (s *Server) adminPage(w http.ResponseWriter, r *http.Request, _ *User) {
	w.Header().Set("Cache-Control", "no-store")
	http.ServeFileFS(w, r, s.assets, "admin.html")
}

type statusWriter struct {
	http.ResponseWriter
	code int
}

func (sw *statusWriter) WriteHeader(code int) {
	if sw.code == 0 {
		sw.code = code
	}
	sw.ResponseWriter.WriteHeader(code)
}

func (sw *statusWriter) Write(b []byte) (int, error) {
	if sw.code == 0 {
		sw.code = 200
	}
	return sw.ResponseWriter.Write(b)
}

func (s *Server) logRequests(next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		start := time.Now()
		sw := &statusWriter{ResponseWriter: w}
		next.ServeHTTP(sw, r)
		if sw.code == 0 {
			sw.code = 200
		}
		// quiet about static files unless something went wrong
		if s.conf.Dev || sw.code >= 400 || path.Ext(r.URL.Path) == "" {
			s.log.Printf("%s %d %s %s %s", r.Method, sw.code, r.URL.Path,
				time.Since(start).Round(time.Microsecond), clientIP(r))
		}
	})
}

func clientIP(r *http.Request) string {
	if ip := r.Header.Get("X-Real-Ip"); ip != "" {
		return ip
	}
	if f := r.Header.Get("X-Forwarded-For"); f != "" {
		return strings.TrimSpace(strings.Split(f, ",")[0])
	}
	host, _, err := net.SplitHostPort(r.RemoteAddr)
	if err != nil {
		return r.RemoteAddr
	}
	return host
}

// limiter allows n events per key per window.
type limiter struct {
	sync.Mutex
	seen map[string][]time.Time
}

func newLimiter() *limiter {
	return &limiter{seen: map[string][]time.Time{}}
}

func (l *limiter) allow(key string, n int, window time.Duration) bool {
	l.Lock()
	defer l.Unlock()
	now := time.Now()
	recent := l.seen[key][:0]
	for _, t := range l.seen[key] {
		if now.Sub(t) < window {
			recent = append(recent, t)
		}
	}
	if len(recent) >= n {
		l.seen[key] = recent
		return false
	}
	l.seen[key] = append(recent, now)
	if len(l.seen) > 10000 {
		// forget everyone rather than grow without end
		l.seen = map[string][]time.Time{key: l.seen[key]}
	}
	return true
}
