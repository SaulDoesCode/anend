package backend

import (
	"errors"
	"html/template"
	"net/http"
	"net/url"
	"strconv"
)

func templateEscape(s string) string {
	return template.HTMLEscapeString(s)
}

func (s *Server) writLink(w *Writ) string {
	return s.conf.BaseURL + "/writ/" + url.PathEscape(w.Slug) + "?writ=" + url.QueryEscape(w.Key)
}

// writPage renders a writ with the writ template. If the slug changed since
// the link was made, the ?writ=key fallback still finds it.
func (s *Server) writPage(w http.ResponseWriter, r *http.Request) {
	q := WritQuery{UpdateViews: true, Slug: r.PathValue("slug")}
	if u, err := s.CurrentUser(w, r); err == nil {
		q.Viewer = u.Key
		q.IncludeMembersOnly = true
	}
	wr, err := s.Store.Writ(q)
	if errors.Is(err, ErrNotFound) {
		key := r.URL.Query().Get("writ")
		if key == "" {
			s.notFound(w, r)
			return
		}
		q.Slug, q.Key = "", key
		wr, err = s.Store.Writ(q)
	}
	if errors.Is(err, ErrNotFound) {
		s.notFound(w, r)
		return
	} else if err != nil {
		sendErr(w, ServerDBError)
		return
	}

	data := obj{
		"key":         wr.Key,
		"title":       wr.Title,
		"author":      wr.Author,
		"description": wr.Description,
		"tags":        wr.Tags,
		"slug":        wr.Slug,
		"views":       wr.Views,
		"content":     template.HTML(wr.Content),
		"injection":   template.HTML(wr.Injection),
		"Created":     wr.Created.Format("2 January 2006"),
		"CreateDate":  wr.Created.Format("2006-01-02T15:04:05Z07:00"),
		"URL":         s.writLink(&wr),
	}
	if len(wr.Edits) > 0 {
		data["ModifiedDate"] = wr.Edits[len(wr.Edits)-1].Format("2006-01-02T15:04:05Z07:00")
	}
	w.Header().Set("Content-Type", "text/html; charset=utf-8")
	if err := s.template().ExecuteTemplate(w, "writ", data); err != nil {
		s.log.Printf("writ template: %v", err)
	}
}

func (s *Server) likeWrit(w http.ResponseWriter, r *http.Request, u *User) {
	liked, err := s.Store.ToggleLike(r.PathValue("slug"), u.Key)
	if errors.Is(err, ErrNotFound) {
		sendErr(w, NoSuchWrit)
		return
	} else if err != nil {
		sendErr(w, ServerDBError)
		return
	}
	msg := "success, writ unliked"
	if liked {
		msg = "success, writ liked!"
	}
	sendJSON(w, 200, obj{"msg": msg, "liked": liked})
}

// listWrits serves /writs/{page}/{count} and /writs-by-tag/{tag}/{page}/{count}:
// public writs, newest first. As with the old AQL, page is the offset.
func (s *Server) listWrits(w http.ResponseWriter, r *http.Request) {
	page, err1 := strconv.ParseInt(r.PathValue("page"), 10, 64)
	count, err2 := strconv.ParseInt(r.PathValue("count"), 10, 64)
	if err1 != nil || err2 != nil || page < 0 || count < 0 {
		sendErr(w, BadRequestError)
		return
	}
	if count > 200 {
		sendErr(w, RequestQueryOverLimitMembers)
		return
	}
	q := WritQuery{Limit: []int64{page, count}}
	if tag := r.PathValue("tag"); tag != "" {
		q.Tags = []string{tag}
	}
	if _, err := s.CurrentUser(w, r); err == nil {
		q.IncludeMembersOnly = true
	} else if count > 50 {
		sendErr(w, RequestQueryOverLimit)
		return
	}
	writs, err := s.Store.Writs(q)
	if err != nil {
		sendErr(w, ServerDBError)
		return
	}
	sendJSON(w, 200, writs)
}

func (s *Server) saveWrit(w http.ResponseWriter, r *http.Request, u *User) {
	var in Writ
	if err := bind(r, &in); err != nil {
		sendErr(w, BadRequestError)
		return
	}
	saved, published, err := s.Store.SaveWrit(in, u)
	switch {
	case errors.Is(err, ErrNotFound):
		sendErr(w, NoSuchWrit)
		return
	case err != nil:
		sendJSON(w, 422, obj{"ok": false, "err": err.Error()})
		return
	}
	s.log.Printf("writ saved: %s (%s) by %s", saved.Title, saved.Key, u.Username)
	if published {
		go s.notifySubscribers(saved)
	}
	sendJSON(w, 203, obj{"msg": "success!", "ok": true, "_key": saved.Key, "slug": saved.Slug})
}

func (s *Server) queryWrits(w http.ResponseWriter, r *http.Request, _ *User) {
	var q WritQuery
	if err := bind(r, &q); err != nil {
		sendErr(w, BadRequestError)
		return
	}
	if q.One {
		wr, err := s.Store.Writ(q)
		if errors.Is(err, ErrNotFound) {
			sendErr(w, NoSuchWrit)
			return
		} else if err != nil {
			sendErr(w, ServerDBError)
			return
		}
		sendJSON(w, 200, wr)
		return
	}
	writs, err := s.Store.Writs(q)
	if err != nil {
		sendErr(w, ServerDBError)
		return
	}
	sendJSON(w, 200, writs)
}

func (s *Server) deleteWrit(w http.ResponseWriter, r *http.Request, u *User) {
	key := r.PathValue("key")
	if err := s.Store.DeleteWrit(key); err != nil {
		sendErr(w, DeleteWritError)
		return
	}
	s.log.Printf("writ %s deleted by %s", key, u.Username)
	sendJSON(w, 200, obj{"msg": "writ deleted, it's gone"})
}
