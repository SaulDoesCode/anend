package backend

import (
	"crypto/tls"
	"errors"
	"fmt"
	"net/smtp"
	"net/textproto"

	"github.com/SaulDoesCode/email"
	trumail "github.com/sdwolfe32/trumail/verifier"
)

// Email - alias for jordan-wright/email Email struct
type Email = email.Email

// EmailSettings - email configuration and setup to send authtokens and stuff
var (
	EmailTLSConfig    *tls.Config
	SMTPAuth          smtp.Auth
	EmailVerifier     trumail.Verifier
	InvalidEmailError = errors.New(`Invalid Email Address`)
)

// Init - initialize the blog's email configuration
func startEmailer() {

	SMTPAuth = smtp.PlainAuth("", EmailConf.Email, EmailConf.Password, EmailConf.Server)

	// TLS config
	EmailTLSConfig = &tls.Config{
		InsecureSkipVerify: true,
		ServerName:         EmailConf.Server,
	}

	EmailVerifier = trumail.NewVerifier(20, Domain, EmailConf.Email)
	fmt.Println(`SMTP Emailer Started`)
}

func isValidEmail(email string) bool {
	if !validEmail(email) {
		return false
	}
	res := EmailVerifier.Verify(email)[0]
	if res.Error != "" {
		return false
	}
	if DevMode {
		fmt.Println("\nVerify Email Status:", email, "hostexists:", res.HostExists, "deliverable:", res.Deliverable, "disposable:", res.Disposable)
	}
	return res.HostExists && !res.Disposable
}

// SendEmail - Send an email using an email struct, with the default SMTP cofiguration
func SendEmail(mail Email) error {
	if mail.From == "" {
		mail.From = EmailConf.FromTxt
	}

	if &mail.Headers == nil {
		mail.Headers = textproto.MIMEHeader{}
	}

	if !isValidEmail(mail.To[0]) {
		return InvalidEmailError
	}
	return mail.SendWithTLS(EmailConf.Address, SMTPAuth, EmailTLSConfig)
}
