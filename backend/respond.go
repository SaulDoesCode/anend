package backend

import (
	"encoding/json"
	"errors"
	"io"
	"mime"
	"net/http"

	"github.com/vmihailenco/msgpack/v5"
)

// HTTPError is an error with a status code, sent as {"err": msg}.
type HTTPError struct {
	Code int
	Msg  string
}

func (e *HTTPError) Error() string { return e.Msg }

func httpErr(code int, msg string) *HTTPError { return &HTTPError{code, msg} }

// the old server's canned responses
var (
	UnauthorizedError            = httpErr(403, "unauthorized request, cannot proceed")
	InvalidDetailsError          = httpErr(401, "invalid details, could not authorize user")
	BadUsernameError             = httpErr(401, "invalid username, could not authorize user")
	BadEmailError                = httpErr(401, "invalid email, could not authorize user")
	BadRequestError              = httpErr(400, "bad request, check details and try again")
	ServerDBError                = httpErr(500, "server error, could not complete your request")
	AlreadyLoggedIn              = httpErr(203, "You're already logged in :D")
	RateLimitingError            = httpErr(429, "ratelimiting: too many auth requests, wait a bit and try again")
	NoSuchWrit                   = httpErr(404, "couldn't find a writ like that")
	RequestQueryOverLimitMembers = httpErr(403, "requesting too many items at once, members >= 200")
	RequestQueryOverLimit        = httpErr(403, "requesting too many items at once, non-members >= 50")
	DeleteWritError              = httpErr(500, "could not delete writ, maybe it didn't exist in the first place")
)

type obj = map[string]any

func sendJSON(w http.ResponseWriter, code int, v any) {
	w.Header().Set("Content-Type", "application/json; charset=utf-8")
	w.WriteHeader(code)
	json.NewEncoder(w).Encode(v)
}

func sendErr(w http.ResponseWriter, err error) {
	var he *HTTPError
	if errors.As(err, &he) {
		sendJSON(w, he.Code, obj{"err": he.Msg})
		return
	}
	sendJSON(w, 500, obj{"err": err.Error()})
}

// bind decodes a request body into v. The admin panel and front page send
// msgpack (via haal), curl and friends send json; both work. msgpack is
// decoded generically and then run through json, so the json field names
// apply either way.
func bind(r *http.Request, v any) error {
	body, err := io.ReadAll(io.LimitReader(r.Body, 8<<20))
	if err != nil {
		return err
	}
	ctype, _, _ := mime.ParseMediaType(r.Header.Get("Content-Type"))
	if ctype == "application/msgpack" || ctype == "application/x-msgpack" {
		var generic any
		if err := msgpack.Unmarshal(body, &generic); err != nil {
			return err
		}
		if body, err = json.Marshal(generic); err != nil {
			return err
		}
	}
	return json.Unmarshal(body, v)
}
