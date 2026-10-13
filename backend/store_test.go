package backend

import (
	"errors"
	"path/filepath"
	"strings"
	"testing"
	"time"

	bolt "go.etcd.io/bbolt"
)

func openTestStore(t *testing.T) *Store {
	t.Helper()
	s, err := Open(filepath.Join(t.TempDir(), "sub", "test.db"))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { s.Close() })
	return s
}

func testAuthor(t *testing.T, s *Store) User {
	t.Helper()
	u, err := s.MakeAdmin("saul@example.com", "saul")
	if err != nil {
		t.Fatal(err)
	}
	return u
}

func TestWritCRUD(t *testing.T) {
	s := openTestStore(t)
	u := testAuthor(t, s)

	if _, _, err := s.SaveWrit(Writ{Title: "No tags", Markdown: "x"}, &u); err != ErrMissingTags {
		t.Fatalf("want ErrMissingTags, got %v", err)
	}
	if _, _, err := s.SaveWrit(Writ{Title: "Nobody", Markdown: "x", Tags: []string{"a"}, Author: "ghost"}, &u); err != ErrAuthorIsNoUser {
		t.Fatalf("want ErrAuthorIsNoUser, got %v", err)
	}

	w, published, err := s.SaveWrit(Writ{Title: "Hello, World!", Markdown: "some *text*", Tags: []string{"go", " a ", "go"}}, &u)
	if err != nil {
		t.Fatal(err)
	}
	if published {
		t.Error("a private writ shouldn't count as published")
	}
	if w.Key == "" || w.Slug != "hello-world" || w.Author != "saul" || w.AuthorKey != u.Key {
		t.Errorf("unexpected writ: %+v", w)
	}
	if !strings.Contains(w.Content, "<em>text</em>") {
		t.Errorf("content not rendered: %q", w.Content)
	}
	if strings.Join(w.Tags, ",") != "go,a" {
		t.Errorf("tags not cleaned: %q", w.Tags)
	}
	if w.Created.IsZero() || len(w.Edits) != 0 {
		t.Errorf("dates: %v %v", w.Created, w.Edits)
	}

	got, err := s.WritByKey(w.Key)
	if err != nil || got.Title != "Hello, World!" {
		t.Fatalf("by key: %v %+v", err, got)
	}

	// update by key: retitle, publish
	w2, published, err := s.SaveWrit(Writ{Key: w.Key, Title: "Hello again", Markdown: "new **body**", Tags: []string{"b"}, Public: true}, &u)
	if err != nil {
		t.Fatal(err)
	}
	if !published {
		t.Error("going public should count as published")
	}
	if w2.Slug != "hello-again" || len(w2.Edits) != 1 || !w2.Created.Equal(w.Created) || !strings.Contains(w2.Content, "<strong>body</strong>") {
		t.Errorf("update: %+v", w2)
	}

	// update by title, without a key
	w3, published, err := s.SaveWrit(Writ{Title: "Hello again", Markdown: "third", Tags: []string{"b"}, Public: true}, &u)
	if err != nil {
		t.Fatal(err)
	}
	if published || w3.Key != w.Key || len(w3.Edits) != 2 {
		t.Errorf("update by title: published=%v %+v", published, w3)
	}
	if s.CountWrits() != 1 {
		t.Errorf("want 1 writ, have %d", s.CountWrits())
	}

	if err := s.DeleteWrit(w.Key); err != nil {
		t.Fatal(err)
	}
	if _, err := s.WritByKey(w.Key); err != ErrNotFound {
		t.Errorf("deleted writ still there: %v", err)
	}
	if _, err := s.WritBySlug("hello-again"); err != ErrNotFound {
		t.Errorf("slug index left behind: %v", err)
	}
	if err := s.DeleteWrit(w.Key); err != ErrNotFound {
		t.Errorf("deleting twice: %v", err)
	}
}

func TestWritIndexes(t *testing.T) {
	s := openTestStore(t)
	u := testAuthor(t, s)

	a, _, _ := s.SaveWrit(Writ{Title: "Same", Markdown: "a", Tags: []string{"x", "y"}, Public: true}, &u)
	// a writ titled differently but slugging the same gets a suffix
	b, _, err := s.SaveWrit(Writ{Title: "same!", Markdown: "b", Tags: []string{"y"}, Public: true}, &u)
	if err != nil {
		t.Fatal(err)
	}
	if a.Slug != "same" || b.Slug != "same-2" {
		t.Errorf("slugs: %q %q", a.Slug, b.Slug)
	}
	if got, _ := s.WritBySlug("same-2"); got.Key != b.Key {
		t.Errorf("slug index: %+v", got)
	}
	if got, _ := s.WritByTitle("Same"); got.Key != a.Key {
		t.Errorf("title index: %+v", got)
	}

	// retagging moves the writ between tag indexes
	if _, _, err := s.SaveWrit(Writ{Key: a.Key, Markdown: "a", Tags: []string{"z"}, Public: true}, &u); err != nil {
		t.Fatal(err)
	}
	if ws, _ := s.Writs(WritQuery{Tags: []string{"x"}}); len(ws) != 0 {
		t.Errorf("old tag still indexed: %v", ws)
	}
	if ws, _ := s.Writs(WritQuery{Tags: []string{"z"}}); len(ws) != 1 || ws[0].Key != a.Key {
		t.Errorf("new tag not indexed: %v", ws)
	}
	if ws, _ := s.Writs(WritQuery{Tags: []string{"y"}}); len(ws) != 1 || ws[0].Key != b.Key {
		t.Errorf("tag y: %v", ws)
	}
}

func TestWritQueries(t *testing.T) {
	s := openTestStore(t)
	base := time.Date(2018, 11, 1, 12, 0, 0, 0, time.UTC)
	add := func(key string, day int, public, members bool, tags ...string) {
		w := Writ{Key: key, Title: "writ " + key, Slug: "writ-" + key, Markdown: "m", Content: "<p>m</p>",
			Tags: tags, Created: base.AddDate(0, 0, day), Public: public, MembersOnly: members, Author: "saul"}
		if err := s.PutWrit(&w); err != nil {
			t.Fatal(err)
		}
	}
	add("1", 0, true, false, "a")
	add("2", 1, true, false, "a", "b")
	add("3", 2, false, false, "a")
	add("4", 3, true, true, "b")
	add("5", 4, true, false, "b")

	keys := func(ws []Writ) string {
		var k []string
		for _, w := range ws {
			k = append(k, w.Key)
		}
		return strings.Join(k, ",")
	}
	cases := []struct {
		name string
		q    WritQuery
		want string
	}{
		{"public, newest first", WritQuery{}, "5,2,1"},
		{"members too", WritQuery{IncludeMembersOnly: true}, "5,4,2,1"},
		{"everything", WritQuery{IncludePrivate: true, IncludeMembersOnly: true}, "5,4,3,2,1"},
		{"private only", WritQuery{PrivateOnly: true}, "3"},
		{"limit", WritQuery{Limit: []int64{2}}, "5,2"},
		{"offset and count", WritQuery{Limit: []int64{1, 1}}, "2"},
		{"offset past the end", WritQuery{Limit: []int64{9, 3}}, ""},
		{"tag", WritQuery{Tags: []string{"a"}}, "2,1"},
		{"all tags", WritQuery{Tags: []string{"a", "b"}}, "2"},
		{"tag and limit", WritQuery{Tags: []string{"b"}, IncludeMembersOnly: true, Limit: []int64{1, 2}}, "4,2"},
		{"between", WritQuery{Between: Timeframe{Start: base, End: base.AddDate(0, 0, 4)}, IncludePrivate: true}, "3,2"},
		{"author", WritQuery{Author: "nobody"}, ""},
	}
	for _, c := range cases {
		ws, err := s.Writs(c.q)
		if err != nil {
			t.Fatalf("%s: %v", c.name, err)
		}
		if got := keys(ws); got != c.want {
			t.Errorf("%s: got %q want %q", c.name, got, c.want)
		}
	}

	// projections, like the old UNSET
	ws, _ := s.Writs(WritQuery{Limit: []int64{1}})
	if ws[0].Markdown != "" || ws[0].Content == "" {
		t.Errorf("reader mode should drop markdown and keep content: %+v", ws[0])
	}
	ws, _ = s.Writs(WritQuery{EditorMode: true, Limit: []int64{1}, Omissions: []string{"tags"}})
	if ws[0].Markdown == "" || ws[0].Content != "" || ws[0].Tags != nil {
		t.Errorf("editor mode should drop content and keep markdown: %+v", ws[0])
	}

	// single writ queries
	if _, err := s.Writ(WritQuery{Key: "3"}); err != ErrNotFound {
		t.Errorf("a private writ shouldn't be found publicly: %v", err)
	}
	w, err := s.Writ(WritQuery{Slug: "writ-2", UpdateViews: true, Viewer: "u1"})
	if err != nil || w.Key != "2" {
		t.Fatalf("by slug: %v %+v", err, w)
	}
	s.Writ(WritQuery{Slug: "writ-2", UpdateViews: true, Viewer: "u1"})
	full, _ := s.WritByKey("2")
	if full.Views != 2 || len(full.ViewedBy) != 1 {
		t.Errorf("views: %d %v", full.Views, full.ViewedBy)
	}
	if w, err := s.Writ(WritQuery{Tags: []string{"b"}, IncludeMembersOnly: true}); err != nil || w.Key != "5" {
		t.Errorf("scan for one: %v %+v", err, w)
	}

	liked, err := s.ToggleLike("writ-1", "u1")
	if err != nil || !liked {
		t.Fatalf("like: %v %v", liked, err)
	}
	if liked, _ := s.ToggleLike("writ-1", "u1"); liked {
		t.Error("second like should unlike")
	}
}

func TestUsersAndTokens(t *testing.T) {
	s := openTestStore(t)

	if _, err := s.CreateUser("not an email", "bob"); err != ErrBadDetails {
		t.Errorf("bad email: %v", err)
	}
	u, err := s.FindOrCreateUser("bob@example.com", "bob")
	if err != nil {
		t.Fatal(err)
	}
	if u.Verified() || u.IsAdmin() || !u.HasRole(UnverifiedUser) {
		t.Errorf("new user roles: %v", u.Roles)
	}
	if again, err := s.FindOrCreateUser("BOB@example.com", "Bob"); err != nil || again.Key != u.Key {
		t.Errorf("same details again: %v %+v", err, again)
	}
	if _, err := s.FindOrCreateUser("bob@example.com", "robert"); err != ErrEmailTaken {
		t.Errorf("email with another username: %v", err)
	}
	if _, err := s.FindOrCreateUser("other@example.com", "BOB"); err != ErrUsernameTaken {
		t.Errorf("username with another email: %v", err)
	}
	if s.IsUsernameAvailable("bob") || !s.IsUsernameAvailable("alice") || s.IsUsernameAvailable("x") {
		t.Error("username availability")
	}

	// magic link: once, and it verifies the user
	link, err := s.NewVerifier(u.Key, time.Minute)
	if err != nil {
		t.Fatal(err)
	}
	v, err := s.UseVerifier(link)
	if err != nil || v.Key != u.Key || !v.Verified() || v.HasRole(UnverifiedUser) {
		t.Fatalf("use verifier: %v %+v", err, v)
	}
	if _, err := s.UseVerifier(link); err != ErrNotFound {
		t.Errorf("a verifier works only once: %v", err)
	}
	old, _ := s.NewVerifier(u.Key, -time.Second)
	if _, err := s.UseVerifier(old); err != ErrNotFound {
		t.Errorf("expired verifier: %v", err)
	}
	if _, err := s.NewVerifier("nobody", time.Minute); err != ErrNotFound {
		t.Errorf("verifier for nobody: %v", err)
	}

	// sessions
	token, err := s.NewSession(u.Key, time.Hour)
	if err != nil {
		t.Fatal(err)
	}
	su, sess, err := s.SessionUser(token)
	if err != nil || su.Key != u.Key || len(su.Logins) != 1 {
		t.Fatalf("session user: %v %+v", err, su)
	}
	if err := s.RenewSession(token, 48*time.Hour); err != nil {
		t.Fatal(err)
	}
	if _, renewed, _ := s.SessionUser(token); !renewed.Expires.After(sess.Expires) {
		t.Error("renew didn't move the expiry")
	}
	if _, _, err := s.SessionUser("made-up"); err != ErrNotFound {
		t.Errorf("unknown token: %v", err)
	}
	s.DeleteSession(token)
	if _, _, err := s.SessionUser(token); err != ErrNotFound {
		t.Errorf("logged out token: %v", err)
	}
	expired, _ := s.NewSession(u.Key, -time.Second)
	if _, _, err := s.SessionUser(expired); err != ErrNotFound {
		t.Errorf("expired session: %v", err)
	}

	// the store only keeps hashes of tokens
	if err := s.db.View(func(tx *bolt.Tx) error {
		if tx.Bucket(bSessions).Get([]byte(token)) != nil {
			return errors.New("raw token stored")
		}
		return nil
	}); err != nil {
		t.Error(err)
	}

	// admin
	a, err := s.MakeAdmin("bob@example.com", "")
	if err != nil || a.Key != u.Key || !a.IsAdmin() {
		t.Errorf("promote: %v %+v", err, a)
	}
	n, err := s.MakeAdmin("new.person+x@example.com", "")
	if err != nil || n.Username != "new.personx" || !n.IsAdmin() || !n.Verified() {
		t.Errorf("new admin: %v %+v", err, n)
	}

	s.UpdateUser(u.Key, func(u *User) error { u.Subscriber = true; return nil })
	if emails, _ := s.SubscriberEmails(); len(emails) != 1 || emails[0] != "bob@example.com" {
		t.Errorf("subscribers: %v", emails)
	}

	pending, _ := s.NewVerifier(u.Key, -time.Second)
	if err := s.Prune(); err != nil {
		t.Fatal(err)
	}
	if _, err := s.UseVerifier(pending); err != ErrNotFound {
		t.Error("prune should remove expired verifiers")
	}
}

func TestImport(t *testing.T) {
	s := openTestStore(t)
	dump := `[
	{"_key":"1000001","_id":"writs/1000001","_rev":"_X","title":"Old","markdown":"hi *there*","tags":["a"],
	 "created":"2018-11-16T20:04:05.12+02:00","edits":["2018-11-17T10:00:00Z"],"public":true,"views":3,"author":"saul"},
	{"_key":"1000002","title":"Draft","markdown":"wip","tags":["b"],"created":"2018-12-01T00:00:00Z"}
]`
	n, err := s.ImportWrits(strings.NewReader(dump))
	if err != nil || n != 2 {
		t.Fatalf("import: %d %v", n, err)
	}
	w, err := s.WritByKey("1000001")
	if err != nil {
		t.Fatal(err)
	}
	if w.Created.Unix() != 1542391445 || len(w.Edits) != 1 || !w.Public || w.Views != 3 || w.Slug != "old" ||
		!strings.Contains(w.Content, "<em>there</em>") {
		t.Errorf("imported writ: %+v", w)
	}
	if ws, _ := s.Writs(WritQuery{}); len(ws) != 1 {
		t.Errorf("only the public writ should list: %v", ws)
	}

	// jsonl, and importing again replaces rather than duplicates
	jsonl := `{"_key":"1000002","title":"Draft","markdown":"done","tags":["b"],"created":"2018-12-01T00:00:00Z","published":true}
{"title":"No key","markdown":"x","tags":["c"],"created":"2019-01-01T00:00:00Z"}
`
	if n, err := s.ImportWrits(strings.NewReader(jsonl)); err != nil || n != 2 {
		t.Fatalf("jsonl import: %d %v", n, err)
	}
	if s.CountWrits() != 3 {
		t.Errorf("want 3 writs, have %d", s.CountWrits())
	}
	if w, _ := s.WritByKey("1000002"); !w.Public || w.Markdown != "done" {
		t.Errorf("reimport: %+v", w)
	}
	// new writs get keys after the imported ones
	u := testAuthor(t, s)
	nw, _, err := s.SaveWrit(Writ{Title: "Fresh", Markdown: "x", Tags: []string{"d"}}, &u)
	if err != nil || nw.Key == "1000001" || nw.Key == "1000002" {
		t.Errorf("new key: %v %q", err, nw.Key)
	}

	if _, err := s.ImportWrits(strings.NewReader(`[{"title":"bad","created":"yesterday"}]`)); err == nil {
		t.Error("a bad date should fail the import")
	}
}

func TestSlugify(t *testing.T) {
	for in, want := range map[string]string{
		"Hello, World!":     "hello-world",
		"  spaces  around ": "spaces-around",
		"Ünïcode Straße":    "ünïcode-straße",
		"!!!":               "writ",
		"a--b__c":           "a-b-c",
	} {
		if got := Slugify(in); got != want {
			t.Errorf("Slugify(%q) = %q, want %q", in, got, want)
		}
	}
}
