package backend

import (
	"bytes"
	"fmt"
	"net/http"
	"strings"
	"time"
)

const (
	sessionTTL  = 7 * 24 * time.Hour
	verifierTTL = 15 * time.Minute
	cookieName  = "Auth"
)

func (s *Server) secureCookies() bool {
	return strings.HasPrefix(s.conf.BaseURL, "https://")
}

func (s *Server) setSessionCookie(w http.ResponseWriter, token string, ttl time.Duration) {
	http.SetCookie(w, &http.Cookie{
		Name:     cookieName,
		Value:    token,
		Path:     "/",
		MaxAge:   int(ttl.Seconds()),
		Expires:  time.Now().Add(ttl),
		HttpOnly: true,
		Secure:   s.secureCookies(),
		SameSite: http.SameSiteLaxMode,
	})
}

// CurrentUser returns the logged in user behind a request's Auth cookie,
// renewing the session when it's within two days of running out.
func (s *Server) CurrentUser(w http.ResponseWriter, r *http.Request) (*User, error) {
	c, err := r.Cookie(cookieName)
	if err != nil || len(c.Value) < 5 {
		return nil, UnauthorizedError
	}
	u, sess, err := s.Store.SessionUser(c.Value)
	if err != nil {
		return nil, UnauthorizedError
	}
	if w != nil && time.Until(sess.Expires) < 48*time.Hour {
		if s.Store.RenewSession(c.Value, sessionTTL) == nil {
			s.setSessionCookie(w, c.Value, sessionTTL)
		}
	}
	return &u, nil
}

// authed wraps a handler that needs a logged in user.
func (s *Server) authed(h func(http.ResponseWriter, *http.Request, *User)) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		u, err := s.CurrentUser(w, r)
		if err != nil {
			sendErr(w, UnauthorizedError)
			return
		}
		h(w, r, u)
	}
}

// admin wraps a handler that needs an admin.
func (s *Server) admin(h func(http.ResponseWriter, *http.Request, *User)) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		u, err := s.CurrentUser(w, r)
		if err != nil || !u.IsAdmin() {
			sendErr(w, UnauthorizedError)
			return
		}
		h(w, r, u)
	}
}

func (s *Server) checkUsername(w http.ResponseWriter, r *http.Request) {
	sendJSON(w, 200, obj{"ok": s.Store.IsUsernameAvailable(r.PathValue("username"))})
}

// LoginLink makes a magic link for a user and hands it over: by email when a
// mailer is set up, otherwise (or if sending fails) by printing it to the
// server's log, so whoever runs the server can open it.
func (s *Server) LoginLink(u *User) (string, error) {
	token, err := s.Store.NewVerifier(u.Key, verifierTTL)
	if err != nil {
		return "", err
	}
	link := s.conf.BaseURL + "/auth/" + token

	if s.conf.Mailer.Configured() {
		err := s.mailLoginLink(u, link)
		if err == nil {
			s.log.Printf("login link for %s <%s> sent by email", u.Username, u.Email)
			return link, nil
		}
		s.log.Printf("couldn't email the login link to %s: %v", u.Email, err)
	}
	s.log.Printf("login link for %s <%s> (good for %s, once):\n\n    %s\n\n", u.Username, u.Email, verifierTTL, link)
	return link, nil
}

func (s *Server) mailLoginLink(u *User, link string) error {
	vars := obj{
		"AppName":  s.conf.AppName,
		"Username": u.Username,
		"Link":     link,
		"Domain":   strings.TrimPrefix(strings.TrimPrefix(s.conf.BaseURL, "https://"), "http://"),
	}
	t := s.template()
	var text, html bytes.Buffer
	if err := t.ExecuteTemplate(&text, "AuthEmailTXT", vars); err != nil {
		return err
	}
	if err := t.ExecuteTemplate(&html, "AuthEmail", vars); err != nil {
		return err
	}
	subject := "Login to " + s.conf.AppName
	if !u.Verified() {
		subject = "Welcome to " + s.conf.AppName
	}
	return s.conf.Mailer.Send([]string{u.Email}, nil, subject, text.String(), html.String())
}

// AuthRequest is the login form: a username and email pair.
type AuthRequest struct {
	Email    string `json:"email"`
	Username string `json:"username"`
}

func (s *Server) requestLogin(w http.ResponseWriter, r *http.Request) {
	if _, err := s.CurrentUser(nil, r); err == nil {
		sendErr(w, AlreadyLoggedIn)
		return
	}
	var req AuthRequest
	if err := bind(r, &req); err != nil {
		sendErr(w, BadRequestError)
		return
	}
	req.Email = strings.TrimSpace(req.Email)
	req.Username = strings.TrimSpace(req.Username)
	if !validEmail(req.Email) {
		sendErr(w, BadEmailError)
		return
	}
	if !validUsername(req.Username) {
		sendErr(w, BadUsernameError)
		return
	}
	if !s.conf.NoRateLimit &&
		(!s.limits.allow("ip:"+clientIP(r), 10, 10*time.Minute) ||
			!s.limits.allow("email:"+strings.ToLower(req.Email), 3, 5*time.Minute)) {
		sendErr(w, RateLimitingError)
		return
	}

	u, err := s.Store.FindOrCreateUser(req.Email, req.Username)
	if err != nil {
		if s.conf.Dev {
			s.log.Printf("login request for %s <%s>: %v", req.Username, req.Email, err)
		}
		sendErr(w, InvalidDetailsError)
		return
	}
	if _, err := s.LoginLink(&u); err != nil {
		sendErr(w, ServerDBError)
		return
	}
	msg := "Thanks " + u.Username + ", the server's admin has your login link."
	if s.conf.Mailer.Configured() {
		msg = "Thanks " + u.Username + ", we sent you an authentication email."
	}
	sendJSON(w, 203, obj{"msg": msg})
}

func (s *Server) login(w http.ResponseWriter, r *http.Request) {
	u, err := s.Store.UseVerifier(r.PathValue("verifier"))
	if err != nil {
		sendErr(w, UnauthorizedError)
		return
	}
	token, err := s.Store.NewSession(u.Key, sessionTTL)
	if err != nil {
		sendErr(w, ServerDBError)
		return
	}
	s.setSessionCookie(w, token, sessionTTL)
	s.log.Printf("%s logged in", u.Username)
	if u.IsAdmin() {
		http.Redirect(w, r, "/admin", http.StatusFound)
		return
	}
	http.Redirect(w, r, "/", http.StatusFound)
}

func (s *Server) logout(w http.ResponseWriter, r *http.Request) {
	if c, err := r.Cookie(cookieName); err == nil && c.Value != "" {
		s.Store.DeleteSession(c.Value)
	}
	http.SetCookie(w, &http.Cookie{
		Name: cookieName, Value: "", Path: "/", MaxAge: -1,
		HttpOnly: true, Secure: s.secureCookies(), SameSite: http.SameSiteLaxMode,
	})
	sendJSON(w, 200, obj{"msg": "logged out"})
}

func (s *Server) subscribeToggle(w http.ResponseWriter, r *http.Request, u *User) {
	updated, err := s.Store.UpdateUser(u.Key, func(u *User) error {
		u.Subscriber = !u.Subscriber
		return nil
	})
	if err != nil {
		sendErr(w, ServerDBError)
		return
	}
	msg := "success, you are unsubscribed and will no longer get (non auth related) emails"
	if updated.Subscriber {
		msg = "success, you are subscribed to new writs"
	}
	sendJSON(w, 203, obj{"msg": msg, "subscriber": updated.Subscriber})
}

// notifySubscribers emails subscribers about a writ that was just published.
func (s *Server) notifySubscribers(wr Writ) {
	if !s.conf.Mailer.Configured() {
		return
	}
	emails, err := s.Store.SubscriberEmails()
	if err != nil || len(emails) == 0 {
		return
	}
	link := s.writLink(&wr)
	html := fmt.Sprintf(`<h4>There's a new writ: %s</h4>
<p><a href="%s">check it out</a></p>
<sub><a href="%s/subscribe-toggle">unsubscribe</a></sub>`,
		templateEscape(wr.Title), link, s.conf.BaseURL)
	text := "There's a new writ: " + wr.Title + "\n" + link + "\n\nunsubscribe: " + s.conf.BaseURL + "/subscribe-toggle\n"
	if err := s.conf.Mailer.Send(nil, emails, "new writ: "+wr.Title, text, html); err != nil {
		s.log.Printf("couldn't notify subscribers: %v", err)
	}
}
