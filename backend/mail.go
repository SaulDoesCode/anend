package backend

import (
	"bytes"
	"crypto/rand"
	"encoding/hex"
	"fmt"
	"mime"
	"mime/multipart"
	"net"
	"net/smtp"
	"net/textproto"
	"strings"
	"time"
)

// Mailer sends email through an SMTP server. It's optional: without one,
// login links are printed to the server's terminal.
type Mailer struct {
	Addr     string // host:port
	Username string
	Password string
	From     string // the sender address
	FromName string
}

// Configured reports whether there's an SMTP server to send through.
func (m *Mailer) Configured() bool {
	return m != nil && m.Addr != "" && m.From != ""
}

// Send sends a mail with a plain text part and, if html isn't empty, an html
// part. Recipients in bcc don't show up in the headers.
func (m *Mailer) Send(to, bcc []string, subject, text, html string) error {
	var buf bytes.Buffer
	from := m.From
	if m.FromName != "" {
		from = mime.QEncoding.Encode("utf-8", m.FromName) + " <" + m.From + ">"
	}
	id := make([]byte, 12)
	rand.Read(id)
	domain := m.From[strings.LastIndexByte(m.From, '@')+1:]

	fmt.Fprintf(&buf, "From: %s\r\n", from)
	if len(to) > 0 {
		fmt.Fprintf(&buf, "To: %s\r\n", strings.Join(to, ", "))
	}
	fmt.Fprintf(&buf, "Subject: %s\r\n", mime.QEncoding.Encode("utf-8", subject))
	fmt.Fprintf(&buf, "Date: %s\r\n", time.Now().Format(time.RFC1123Z))
	fmt.Fprintf(&buf, "Message-Id: <%s@%s>\r\n", hex.EncodeToString(id), domain)
	buf.WriteString("MIME-Version: 1.0\r\n")

	if html == "" {
		buf.WriteString("Content-Type: text/plain; charset=utf-8\r\n\r\n")
		buf.WriteString(text)
	} else {
		mw := multipart.NewWriter(&buf)
		fmt.Fprintf(&buf, "Content-Type: multipart/alternative; boundary=%s\r\n\r\n", mw.Boundary())
		for _, part := range []struct{ ctype, body string }{{"text/plain", text}, {"text/html", html}} {
			pw, err := mw.CreatePart(textproto.MIMEHeader{"Content-Type": {part.ctype + "; charset=utf-8"}})
			if err != nil {
				return err
			}
			pw.Write([]byte(part.body))
		}
		mw.Close()
	}

	var auth smtp.Auth
	if m.Username != "" {
		host, _, _ := net.SplitHostPort(m.Addr)
		auth = smtp.PlainAuth("", m.Username, m.Password, host)
	}
	return smtp.SendMail(m.Addr, auth, m.From, append(append([]string{}, to...), bcc...), buf.Bytes())
}
