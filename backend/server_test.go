package backend

import (
	"bytes"
	"encoding/json"
	"io"
	"log"
	"net/http"
	"net/http/cookiejar"
	"net/http/httptest"
	"os"
	"regexp"
	"strings"
	"sync"
	"testing"

	"github.com/vmihailenco/msgpack/v5"
)

type syncBuffer struct {
	sync.Mutex
	bytes.Buffer
}

func (b *syncBuffer) Write(p []byte) (int, error) {
	b.Lock()
	defer b.Unlock()
	return b.Buffer.Write(p)
}

func (b *syncBuffer) String() string {
	b.Lock()
	defer b.Unlock()
	return b.Buffer.String()
}

// TestLoginFlow goes through the whole thing over http: ask for a magic
// link, pick it out of the server's log, follow it, write a writ through the
// admin api and see it listed for the front page.
func TestLoginFlow(t *testing.T) {
	store := openTestStore(t)
	logs := &syncBuffer{}
	ts := httptest.NewUnstartedServer(nil)
	srv, err := NewServer(store, Config{
		BaseURL: "http://" + ts.Listener.Addr().String(),
		Log:     log.New(logs, "", 0),
	}, os.DirFS("../assets"), os.DirFS("../templates"))
	if err != nil {
		t.Fatal(err)
	}
	ts.Config.Handler = srv.Handler()
	ts.Start()
	defer ts.Close()

	jar, _ := cookiejar.New(nil)
	client := &http.Client{Jar: jar, CheckRedirect: func(*http.Request, []*http.Request) error {
		return http.ErrUseLastResponse
	}}
	do := func(method, path, ctype string, body []byte) (int, string) {
		t.Helper()
		req, _ := http.NewRequest(method, ts.URL+path, bytes.NewReader(body))
		if ctype != "" {
			req.Header.Set("Content-Type", ctype)
		}
		res, err := client.Do(req)
		if err != nil {
			t.Fatal(err)
		}
		defer res.Body.Close()
		b, _ := io.ReadAll(res.Body)
		return res.StatusCode, string(b)
	}
	postJSON := func(path string, v any) (int, string) {
		b, _ := json.Marshal(v)
		return do("POST", path, "application/json", b)
	}

	if code, body := do("GET", "/", "", nil); code != 200 || !strings.Contains(body, "<main></main>") {
		t.Fatalf("front page: %d", code)
	}
	if code, _ := do("GET", "/css/index.css", "", nil); code != 200 {
		t.Fatalf("css: %d", code)
	}
	if code, body := do("GET", "/nope", "", nil); code != 404 || !strings.Contains(body, "<html") {
		t.Fatalf("404 page: %d", code)
	}
	if code, _ := do("GET", "/admin", "", nil); code != 403 {
		t.Fatalf("admin without login: %d", code)
	}

	// the first admin, made like `anend-server -admin`
	admin, err := store.MakeAdmin("saul@example.com", "saul")
	if err != nil {
		t.Fatal(err)
	}

	// the front page's login form sends msgpack
	body, _ := msgpack.Marshal(map[string]string{"email": admin.Email, "username": "saul"})
	if code, out := do("POST", "/auth", "application/msgpack", body); code != 203 {
		t.Fatalf("auth request: %d %s", code, out)
	}
	link := regexp.MustCompile(`http://\S+/auth/\S+`).FindString(logs.String())
	if link == "" {
		t.Fatalf("no login link in the log:\n%s", logs.String())
	}
	path := strings.TrimPrefix(link, ts.URL)

	code, _ := do("GET", path, "", nil)
	if code != 302 {
		t.Fatalf("following the link: %d", code)
	}
	if code, _ := do("GET", path, "", nil); code != 403 {
		t.Fatalf("a link works once, got %d", code)
	}
	if code, body := do("GET", "/admin", "", nil); code != 200 || !strings.Contains(body, "markdown-editor") {
		t.Fatalf("admin page after login: %d", code)
	}

	if code, out := postJSON("/writ", map[string]any{
		"title": "First light", "markdown": "it *works*", "tags": []string{"news"}, "public": true,
	}); code != 203 {
		t.Fatalf("save writ: %d %s", code, out)
	}

	// the admin panel's query, in msgpack
	q, _ := msgpack.Marshal(map[string]any{"editormode": true, "includeprivate": true, "includemembersonly": true})
	code, out := do("POST", "/writ-query", "application/msgpack", q)
	var writs []Writ
	json.Unmarshal([]byte(out), &writs)
	if code != 200 || len(writs) != 1 || writs[0].Markdown != "it *works*" {
		t.Fatalf("writ query: %d %s", code, out)
	}

	// logged out, the front page's list shows it
	anon := &http.Client{}
	res, err := anon.Get(ts.URL + "/writs/0/15")
	if err != nil {
		t.Fatal(err)
	}
	writs = nil
	json.NewDecoder(res.Body).Decode(&writs)
	res.Body.Close()
	if len(writs) != 1 || writs[0].Title != "First light" || !strings.Contains(writs[0].Content, "<em>works</em>") || writs[0].Markdown != "" {
		t.Fatalf("front page list: %+v", writs)
	}
	res, _ = anon.Get(ts.URL + "/writ/first-light")
	page, _ := io.ReadAll(res.Body)
	res.Body.Close()
	if res.StatusCode != 200 || !strings.Contains(string(page), "<em>works</em>") || !strings.Contains(string(page), "<h1>First light</h1>") {
		t.Fatalf("writ page: %d %s", res.StatusCode, page)
	}
	if res, _ := anon.Get(ts.URL + "/writ-delete/" + writs[0].Key); res.StatusCode != 403 {
		t.Fatalf("anonymous delete: %d", res.StatusCode)
	}

	if code, _ := do("GET", "/writ-delete/"+writs[0].Key, "", nil); code != 200 {
		t.Fatalf("delete: %d", code)
	}
	if code, _ := do("GET", "/logout", "", nil); code != 200 {
		t.Fatalf("logout: %d", code)
	}
	if code, _ := do("GET", "/admin", "", nil); code != 403 {
		t.Fatalf("admin after logout: %d", code)
	}
}
