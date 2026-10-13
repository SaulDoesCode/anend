package backend

import (
	"bytes"
	"encoding/json"
	"errors"
	"fmt"
	"slices"
	"sort"
	"strings"
	"time"

	bolt "go.etcd.io/bbolt"
)

// Writ is a post: markdown with a title, tags, dates, an edit history and a
// public flag. The json names are the ones the old arangodb documents used.
type Writ struct {
	Key         string      `json:"_key,omitempty"`
	Type        string      `json:"type,omitempty"`
	Title       string      `json:"title,omitempty"`
	AuthorKey   string      `json:"authorkey,omitempty"`
	Author      string      `json:"author,omitempty"`
	Content     string      `json:"content,omitempty"`
	Injection   string      `json:"injection,omitempty"`
	Markdown    string      `json:"markdown,omitempty"`
	Description string      `json:"description,omitempty"`
	Slug        string      `json:"slug,omitempty"`
	Tags        []string    `json:"tags,omitempty"`
	Edits       []time.Time `json:"edits,omitempty"`
	Created     time.Time   `json:"created,omitzero"`
	Views       int64       `json:"views,omitempty"`
	ViewedBy    []string    `json:"viewedby,omitempty"`
	LikedBy     []string    `json:"likedby,omitempty"`
	Public      bool        `json:"public,omitempty"`
	MembersOnly bool        `json:"membersonly,omitempty"`
	NoComments  bool        `json:"nocomments,omitempty"`
	Roles       []int64     `json:"roles,omitempty"`
}

var (
	// ErrMissingTags writ is missing tags
	ErrMissingTags = errors.New("writ doesn't have any tags, add some")
	// ErrIncompleteWrit a new writ needs a title, markdown and an author
	ErrIncompleteWrit = errors.New("a writ needs a title, markdown and an author")
	// ErrAuthorIsNoUser writ's author is persona non grata
	ErrAuthorIsNoUser = errors.New("writ author is not a registered user")
)

// RenderContent renders .Markdown into .Content.
func (w *Writ) RenderContent() {
	w.Content = renderMarkdown(w.Markdown)
}

// ModifiedDate is the time of the last edit, or the creation time.
func (w *Writ) ModifiedDate() time.Time {
	if len(w.Edits) > 0 {
		return w.Edits[len(w.Edits)-1]
	}
	return w.Created
}

// without returns a copy of the writ with the named (json) fields cleared,
// like AQL's UNSET.
func (w Writ) without(fields ...string) Writ {
	for _, f := range fields {
		switch f {
		case "type":
			w.Type = ""
		case "title":
			w.Title = ""
		case "authorkey":
			w.AuthorKey = ""
		case "author":
			w.Author = ""
		case "content":
			w.Content = ""
		case "injection":
			w.Injection = ""
		case "markdown":
			w.Markdown = ""
		case "description":
			w.Description = ""
		case "slug":
			w.Slug = ""
		case "tags":
			w.Tags = nil
		case "edits":
			w.Edits = nil
		case "created":
			w.Created = time.Time{}
		case "views":
			w.Views = 0
		case "viewedby":
			w.ViewedBy = nil
		case "likedby":
			w.LikedBy = nil
		case "public":
			w.Public = false
		case "membersonly":
			w.MembersOnly = false
		case "nocomments":
			w.NoComments = false
		case "roles":
			w.Roles = nil
		}
	}
	return w
}

func dateIndexKey(w *Writ) []byte {
	return []byte(w.Created.UTC().Format("20060102150405.000000000") + "\x00" + w.Key)
}

func tagIndexKey(tag, key string) []byte {
	return []byte(tag + "\x00" + key)
}

// unindexWrit removes the index entries of a stored writ.
func unindexWrit(tx *bolt.Tx, w *Writ) error {
	if k, ok := getIndex(tx, bWritsBySlug, w.Slug); ok && k == w.Key {
		if err := tx.Bucket(bWritsBySlug).Delete([]byte(w.Slug)); err != nil {
			return err
		}
	}
	if k, ok := getIndex(tx, bWritsByTitle, w.Title); ok && k == w.Key {
		if err := tx.Bucket(bWritsByTitle).Delete([]byte(w.Title)); err != nil {
			return err
		}
	}
	for _, tag := range w.Tags {
		if err := tx.Bucket(bWritsByTag).Delete(tagIndexKey(tag, w.Key)); err != nil {
			return err
		}
	}
	return tx.Bucket(bWritsByDate).Delete(dateIndexKey(w))
}

// putWrit stores a writ and its index entries, replacing an older version.
func putWrit(tx *bolt.Tx, w *Writ) error {
	if w.Key == "" {
		return errors.New("writ has no key")
	}
	var old Writ
	switch err := getJSON(tx, bWrits, w.Key, &old); err {
	case nil:
		if err := unindexWrit(tx, &old); err != nil {
			return err
		}
	case ErrNotFound:
	default:
		return err
	}
	if w.Slug != "" {
		if k, ok := getIndex(tx, bWritsBySlug, w.Slug); ok && k != w.Key {
			return fmt.Errorf("slug %q is taken by writ %s", w.Slug, k)
		}
		if err := tx.Bucket(bWritsBySlug).Put([]byte(w.Slug), []byte(w.Key)); err != nil {
			return err
		}
	}
	if w.Title != "" {
		if err := tx.Bucket(bWritsByTitle).Put([]byte(w.Title), []byte(w.Key)); err != nil {
			return err
		}
	}
	for _, tag := range w.Tags {
		if err := tx.Bucket(bWritsByTag).Put(tagIndexKey(tag, w.Key), nil); err != nil {
			return err
		}
	}
	if err := tx.Bucket(bWritsByDate).Put(dateIndexKey(w), nil); err != nil {
		return err
	}
	return putJSON(tx, bWrits, w.Key, w)
}

// uniqueSlug returns base, or base-2, base-3... whichever isn't used by
// another writ.
func uniqueSlug(tx *bolt.Tx, base, key string) string {
	slug := base
	for i := 2; ; i++ {
		k, ok := getIndex(tx, bWritsBySlug, slug)
		if !ok || k == key {
			return slug
		}
		slug = fmt.Sprintf("%s-%d", base, i)
	}
}

func cleanTags(tags []string) []string {
	out := []string{}
	for _, t := range tags {
		t = strings.TrimSpace(t)
		if t != "" && !slices.Contains(out, t) {
			out = append(out, t)
		}
	}
	return out
}

// PutWrit stores a writ as it is (keys, slug and dates included), giving it a
// key if it has none.
func (s *Store) PutWrit(w *Writ) error {
	return s.db.Update(func(tx *bolt.Tx) error {
		if w.Key == "" {
			key, err := nextKey(tx, bWrits)
			if err != nil {
				return err
			}
			w.Key = key
		}
		if w.Created.IsZero() {
			w.Created = time.Now()
		}
		return putWrit(tx, w)
	})
}

// WritByKey gets a writ by its key.
func (s *Store) WritByKey(key string) (Writ, error) {
	var w Writ
	err := s.db.View(func(tx *bolt.Tx) error {
		return getJSON(tx, bWrits, key, &w)
	})
	return w, err
}

// WritBySlug gets a writ by its slug.
func (s *Store) WritBySlug(slug string) (Writ, error) {
	return s.writByIndex(bWritsBySlug, slug)
}

// WritByTitle gets a writ by its exact title.
func (s *Store) WritByTitle(title string) (Writ, error) {
	return s.writByIndex(bWritsByTitle, title)
}

func (s *Store) writByIndex(bucket []byte, value string) (Writ, error) {
	var w Writ
	err := s.db.View(func(tx *bolt.Tx) error {
		key, ok := getIndex(tx, bucket, value)
		if !ok {
			return ErrNotFound
		}
		return getJSON(tx, bWrits, key, &w)
	})
	return w, err
}

// DeleteWrit removes a writ and its index entries.
func (s *Store) DeleteWrit(key string) error {
	return s.db.Update(func(tx *bolt.Tx) error {
		var w Writ
		if err := getJSON(tx, bWrits, key, &w); err != nil {
			return err
		}
		if err := unindexWrit(tx, &w); err != nil {
			return err
		}
		return tx.Bucket(bWrits).Delete([]byte(key))
	})
}

// UpdateWrit changes a stored writ in place, keeping its indexes in step.
func (s *Store) UpdateWrit(key string, change func(*Writ) error) (Writ, error) {
	var w Writ
	err := s.db.Update(func(tx *bolt.Tx) error {
		if err := getJSON(tx, bWrits, key, &w); err != nil {
			return err
		}
		if err := change(&w); err != nil {
			return err
		}
		w.Key = key
		return putWrit(tx, &w)
	})
	return w, err
}

// SaveWrit creates or updates a writ the way the admin editor expects: a writ
// with a key (or with the title of an existing writ) is updated and gets an
// edit time, anything else is created. author is the user saving it. It
// returns the stored writ and whether it was just made public.
func (s *Store) SaveWrit(in Writ, author *User) (Writ, bool, error) {
	in.Tags = cleanTags(in.Tags)
	if len(in.Tags) == 0 {
		return in, false, ErrMissingTags
	}
	in.Title = strings.TrimSpace(in.Title)

	var out Writ
	published := false
	err := s.db.Update(func(tx *bolt.Tx) error {
		var cur Writ
		exists := false
		if in.Key != "" {
			if err := getJSON(tx, bWrits, in.Key, &cur); err != nil {
				return err
			}
			exists = true
		} else if key, ok := getIndex(tx, bWritsByTitle, in.Title); ok && in.Title != "" {
			if err := getJSON(tx, bWrits, key, &cur); err != nil {
				return err
			}
			exists = true
		}

		if !exists {
			if in.Title == "" || in.Markdown == "" {
				return ErrIncompleteWrit
			}
			w := in
			w.Created = time.Now()
			w.Edits = nil
			if w.Author == "" && author != nil {
				w.Author = author.Username
			}
			if w.Author == "" {
				return ErrIncompleteWrit
			}
			var u User
			if err := userByIndex(tx, bUsersByName, w.Author, &u); err != nil {
				return ErrAuthorIsNoUser
			}
			w.AuthorKey = u.Key
			key, err := nextKey(tx, bWrits)
			if err != nil {
				return err
			}
			w.Key = key
			w.RenderContent()
			if w.Slug == "" {
				w.Slug = Slugify(w.Title)
			}
			w.Slug = uniqueSlug(tx, w.Slug, w.Key)
			published = w.Public
			out = w
			return putWrit(tx, &out)
		}

		w := cur
		if in.Title != "" && in.Title != cur.Title {
			w.Title = in.Title
			w.Slug = uniqueSlug(tx, Slugify(in.Title), w.Key)
		}
		if in.Markdown != "" && in.Markdown != cur.Markdown {
			w.Markdown = in.Markdown
			w.RenderContent()
		}
		w.Tags = in.Tags
		if in.Description != "" {
			w.Description = in.Description
		}
		w.Injection = in.Injection
		w.Public = in.Public
		w.MembersOnly = in.MembersOnly
		w.NoComments = in.NoComments
		if in.Type != "" {
			w.Type = in.Type
		}
		if len(in.Roles) > 0 {
			w.Roles = in.Roles
		}
		w.Edits = append(w.Edits, time.Now())
		published = !cur.Public && w.Public
		out = w
		return putWrit(tx, &out)
	})
	return out, published, err
}

// AddView counts a view of a writ, remembering the viewer if there is one.
func (s *Store) AddView(key, viewer string) error {
	_, err := s.UpdateWrit(key, func(w *Writ) error {
		w.Views++
		if viewer != "" && !slices.Contains(w.ViewedBy, viewer) {
			w.ViewedBy = append(w.ViewedBy, viewer)
		}
		return nil
	})
	return err
}

// ToggleLike likes or unlikes a writ for a user, reporting whether it's now liked.
func (s *Store) ToggleLike(slug, user string) (bool, error) {
	w, err := s.WritBySlug(slug)
	if err != nil {
		return false, err
	}
	liked := false
	_, err = s.UpdateWrit(w.Key, func(w *Writ) error {
		if i := slices.Index(w.LikedBy, user); i >= 0 {
			w.LikedBy = slices.Delete(w.LikedBy, i, i+1)
		} else {
			w.LikedBy = append(w.LikedBy, user)
			liked = true
		}
		return nil
	})
	return liked, err
}

// Timeframe a distance of time between a .Start time and a .Finish time
type Timeframe struct {
	Start time.Time `json:"start,omitzero"`
	End   time.Time `json:"end,omitzero"`
}

// WritQuery describes which writs to fetch and how much of them to show. It
// was compiled to AQL by the old server; now it runs as Go over the indexes.
type WritQuery struct {
	One                bool      `json:"one,omitempty"`
	Key                string    `json:"_key,omitempty"`
	PrivateOnly        bool      `json:"privateonly,omitempty"`
	IncludePrivate     bool      `json:"includeprivate,omitempty"`
	EditorMode         bool      `json:"editormode,omitempty"`
	Extensive          bool      `json:"extensive,omitempty"`
	UpdateViews        bool      `json:"updateviews,omitempty"`
	MembersOnly        bool      `json:"membersonly,omitempty"`
	IncludeMembersOnly bool      `json:"includemembersonly,omitempty"`
	DontSort           bool      `json:"dontsort,omitempty"`
	ViewedBy           string    `json:"viewedby,omitempty"`
	LikedBy            string    `json:"likedby,omitempty"`
	Viewer             string    `json:"viewer,omitempty"`
	Title              string    `json:"title,omitempty"`
	Slug               string    `json:"slug,omitempty"`
	Author             string    `json:"author,omitempty"`
	Created            time.Time `json:"created,omitzero"`
	Between            Timeframe `json:"between,omitzero"`
	Roles              []int64   `json:"roles,omitempty"`
	Limit              []int64   `json:"limit,omitempty"`
	Tags               []string  `json:"tags,omitempty"`
	Omissions          []string  `json:"omissions,omitempty"`
}

func containsAll[T comparable](have, want []T) bool {
	for _, x := range want {
		if !slices.Contains(have, x) {
			return false
		}
	}
	return true
}

// Match is the query's FILTER.
func (q *WritQuery) Match(w *Writ) bool {
	switch {
	case q.PrivateOnly && w.Public:
		return false
	case !q.PrivateOnly && !q.IncludePrivate && !w.Public:
		return false
	case q.MembersOnly && !w.MembersOnly:
		return false
	case !q.MembersOnly && !q.IncludeMembersOnly && w.MembersOnly:
		return false
	case !q.Between.Start.IsZero() && !w.Created.After(q.Between.Start):
		return false
	case !q.Between.End.IsZero() && !w.Created.Before(q.Between.End):
		return false
	case !q.Created.IsZero() && !w.Created.Equal(q.Created):
		return false
	case q.Key != "" && w.Key != q.Key:
		return false
	case q.Slug != "" && w.Slug != q.Slug:
		return false
	case q.Title != "" && w.Title != q.Title:
		return false
	case q.Author != "" && w.Author != q.Author:
		return false
	case q.ViewedBy != "" && !slices.Contains(w.ViewedBy, q.ViewedBy):
		return false
	case q.LikedBy != "" && !slices.Contains(w.LikedBy, q.LikedBy):
		return false
	case !containsAll(w.Roles, q.Roles):
		return false
	case !containsAll(w.Tags, q.Tags):
		return false
	}
	return true
}

// project is the query's RETURN UNSET(writ, omissions).
func (q *WritQuery) project(w Writ) Writ {
	omit := append([]string{}, q.Omissions...)
	if q.EditorMode {
		omit = append(omit, "content")
	} else {
		omit = append(omit, "markdown", "edits", "public", "roles", "authorkey")
	}
	if !q.Extensive {
		omit = append(omit, "likedby", "viewedby")
	}
	return w.without(omit...)
}

// Writs runs a query and returns the matching writs, newest first unless
// DontSort is set. Limit is AQL's LIMIT: [count] or [offset, count].
func (s *Store) Writs(q WritQuery) ([]Writ, error) {
	if q.One {
		w, err := s.Writ(q)
		if err != nil {
			return []Writ{}, err
		}
		return []Writ{w}, nil
	}
	var offset, count int64 = 0, -1
	switch len(q.Limit) {
	case 1:
		count = q.Limit[0]
	case 2:
		offset, count = q.Limit[0], q.Limit[1]
	}

	matches := []Writ{}
	err := s.db.View(func(tx *bolt.Tx) error {
		writs := tx.Bucket(bWrits)
		consider := func(key []byte) error {
			raw := writs.Get(key)
			if raw == nil {
				return nil
			}
			var w Writ
			if err := json.Unmarshal(raw, &w); err != nil {
				return err
			}
			if q.Match(&w) {
				matches = append(matches, w)
			}
			return nil
		}

		if len(q.Tags) > 0 {
			// walk the first tag's index, then sort by date
			prefix := []byte(q.Tags[0] + "\x00")
			c := tx.Bucket(bWritsByTag).Cursor()
			for k, _ := c.Seek(prefix); k != nil && bytes.HasPrefix(k, prefix); k, _ = c.Next() {
				if err := consider(k[len(prefix):]); err != nil {
					return err
				}
			}
			if !q.DontSort {
				sort.SliceStable(matches, func(i, j int) bool {
					return matches[i].Created.After(matches[j].Created)
				})
			}
			return nil
		}

		// the date index is already in order, walk it newest first and stop
		// as soon as the page is full
		c := tx.Bucket(bWritsByDate).Cursor()
		for k, _ := c.Last(); k != nil; k, _ = c.Prev() {
			if count >= 0 && int64(len(matches)) >= offset+count {
				break
			}
			i := bytes.IndexByte(k, 0)
			if err := consider(k[i+1:]); err != nil {
				return err
			}
		}
		return nil
	})
	if err != nil {
		return []Writ{}, err
	}

	if offset > int64(len(matches)) {
		offset = int64(len(matches))
	}
	matches = matches[offset:]
	if count >= 0 && count < int64(len(matches)) {
		matches = matches[:count]
	}
	for i := range matches {
		matches[i] = q.project(matches[i])
	}
	return matches, nil
}

// Writ runs a query for a single writ, using the key, slug or title index
// when the query has one. With UpdateViews it also counts the view.
func (s *Store) Writ(q WritQuery) (Writ, error) {
	var w Writ
	var err error
	switch {
	case q.Key != "":
		w, err = s.WritByKey(q.Key)
	case q.Slug != "":
		w, err = s.WritBySlug(q.Slug)
	case q.Title != "":
		w, err = s.WritByTitle(q.Title)
	default:
		var all []Writ
		all, err = s.Writs(WritQuery{
			IncludePrivate: true, IncludeMembersOnly: true, EditorMode: true, Extensive: true,
		})
		if err != nil {
			return Writ{}, err
		}
		err = ErrNotFound
		for _, c := range all {
			if q.Match(&c) {
				w, err = s.WritByKey(c.Key)
				break
			}
		}
	}
	if err != nil {
		return Writ{}, err
	}
	if !q.Match(&w) {
		return Writ{}, ErrNotFound
	}
	if q.UpdateViews {
		if err := s.AddView(w.Key, q.Viewer); err != nil {
			return Writ{}, err
		}
		w.Views++
	}
	return q.project(w), nil
}

// CountWrits returns how many writs there are.
func (s *Store) CountWrits() int {
	n := 0
	s.db.View(func(tx *bolt.Tx) error {
		n = tx.Bucket(bWrits).Stats().KeyN
		return nil
	})
	return n
}
