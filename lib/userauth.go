package backend

import (
	"bytes"
	"crypto/sha1"
	"encoding/hex"
	"errors"
	"fmt"
	"html/template"
	"time"

	jwt "github.com/dgrijalva/jwt-go"
	"github.com/tidwall/gjson"
)

/*
  Validation Code refers to the Code sent to a user via email
  which is used to validate a new login,
  it's part of the no-password system.
*/

var (
	VerificationEmail *template.Template
	Welcome           = struct {
		Verified   string
		Unverified string
	}{}

	Roles = struct {
		Admin int64
		User  int64
	}{
		0,
		1,
	}

	ValidationCodeError = errors.New("Invalid validation code")

	InvalidDetails     = errors.New(`Invalid User Details`)
	UserNotUnique      = errors.New(`User details must be valid and unique`)
	UnexpectedResponse = errors.New(`DB Query returned an unexpected response`)
	UserDoesNotExist   = errors.New(`User does not exist or has missing UID field`)

	TKerrUserDoesNotExist = errors.New("this user does not exist, invalid token")
	TKerrFakeOrBad        = errors.New("outdated or fake, invalid token")
	TKerrUnauthorized     = errors.New("this user is not authorized to use this resource, invalid token")
)

type Verifier struct {
	Code   string    `json:"code"`
	Expiry time.Time `json:"expiry"`
}

type jwtClaim struct {
	UID string `json:"usr"`
	ID  string `json:"id"`
	jwt.StandardClaims
}

func ValidateToken(tokenStr string) (*User, error) {
	token, err := jwt.ParseWithClaims(tokenStr, &jwtClaim{}, func(token *jwt.Token) (interface{}, error) {
		return JWTKey, nil
	})
	if err != nil {
		return nil, err
	}

	if claims, ok := token.Claims.(*jwtClaim); ok {
		user, err := UserByUID(claims.UID)
		if err != nil {
			return nil, TKerrUserDoesNotExist
		}

		if !stringInSlice(tokenStr, user.Tokens) {
			if !token.Valid {
				user.Tokens = removeStringInSlice(user.Tokens, tokenStr)
				_, err = DB.Delete(obj{
					"uid":    user.UID,
					"tokens": nil,
				})
				if err == nil {
					err = user.Update(obj{"tokens": user.Tokens})
				}
				if err != nil {
					return &user, err
				}
			}
			return &user, TKerrFakeOrBad
		}
		return &user, nil
	}
	return nil, TKerrFakeOrBad
}

func DestroyToken(tokenStr string) error {
	token, err := jwt.ParseWithClaims(tokenStr, &jwtClaim{}, func(token *jwt.Token) (interface{}, error) {
		return JWTKey, nil
	})
	if err != nil {
		return err
	}

	if claims, ok := token.Claims.(*jwtClaim); ok {
		user, err := UserByUID(claims.UID)
		if err != nil {
			return TKerrUserDoesNotExist
		}
		_, err = DB.Delete(obj{
			"uid":    user.UID,
			"tokens": nil,
		})
		if err == nil {
			err = user.Update(obj{
				"tokens": removeStringInSlice(user.Tokens, tokenStr),
			})
		}
		return err
	}
	return TKerrFakeOrBad
}

var UserSchema = `
  username: string @index(fulltext, term) @count .
  email: string  @index(exact) @upsert .
  tokens: [string] @index(exact) @count .
  logins: [datetime] @index(hour) @count .
  created: datetime @index(hour) @count .
  verified: bool .
  verifier: string @index(exact) .
  vexpiry: datetime .
`

type User struct {
	UID         string      `json:"uid,omitempty"`
	Name        string      `json:"username,omitempty"`
	Email       string      `json:"email,omitempty"`
	Description string      `json:"description,omitempty"`
	Created     time.Time   `json:"created,omitempty"`
	Logins      []time.Time `json:"logins,omitempty"`
	Tokens      []string    `json:"tokens,omitempty"`
	Roles       []int64     `json:"roles,omitempty"`
	Verified    bool        `json:"verified,omitempty"`
	Verifier    string      `json:"verifier,omitempty"`
	Vexpiry     time.Time   `json:"vexpiry,omitempty"`
}

func (user *User) EmailMD5() string {
	return MD5Hash([]byte(user.Email))
}

func (user *User) GenerateToken(tokenID string) (string, error) {
	sha1TID := sha1.Sum([]byte(tokenID))
	tokenID = hex.EncodeToString(sha1TID[:])

	expiration := time.Now().Add(time.Hour * 72)
	claims := jwtClaim{
		user.UID,
		tokenID,
		jwt.StandardClaims{
			ExpiresAt: expiration.Unix(),
			IssuedAt:  time.Now().Unix(),
			Issuer:    AppName,
		},
	}
	token := jwt.NewWithClaims(jwt.SigningMethodHS256, claims)
	jwtTK, err := token.SignedString(JWTKey)
	if err != nil {
		return "", err
	}

	if len(user.Tokens) > 0 {
		for _, oldTokenStr := range user.Tokens {
			oldToken, err := jwt.ParseWithClaims(oldTokenStr, &jwtClaim{}, func(token *jwt.Token) (interface{}, error) {
				return JWTKey, nil
			})

			if err == nil {
				if claims, ok := oldToken.Claims.(*jwtClaim); ok {
					if claims.ID == tokenID {
						removeStringInSlice(user.Tokens, oldTokenStr)
					}
				}
			}
		}
	}

	user.Tokens = append(user.Tokens, jwtTK)
	user.Update(obj{"tokens": user.Tokens})
	return jwtTK, nil
}

func (user *User) Update(data map[string]interface{}) error {
	if len(user.UID) < 1 {
		return UserDoesNotExist
	}
	data["uid"] = user.UID
	_, err := DB.Mutate(data)
	return err
}

func (user *User) Sync() error {
	res, err := DB.QueryNoVars(`{
    users(func: eq(username, "` + user.Name + `")) {
      uid
      created
      logins
      tokens
			roles
      verified
      verifier
      vexpiry
    }
  }`)
	if err == nil {
		users := gjson.GetBytes(res.GetJson(), "users").Array()
		if len(users) == 1 {
			userJSON := users[0]
			user.UID = userJSON.Get("uid").String()
			user.Description = userJSON.Get("description").String()
			user.Created = userJSON.Get("created").Time()
			user.Verified = userJSON.Get("verified").Bool()
			user.Verifier = userJSON.Get("verifier").String()
			user.Vexpiry = userJSON.Get("vexpiry").Time()

			user.Roles = []int64{}
			userJSON.Get("roles").ForEach(func(key, value gjson.Result) bool {
				user.Roles = append(user.Roles, value.Int())
				return true
			})

			user.Tokens = []string{}
			userJSON.Get("tokens").ForEach(func(key, value gjson.Result) bool {
				user.Tokens = append(user.Tokens, value.String())
				return true
			})

			user.Logins = []time.Time{}
			userJSON.Get("logins").ForEach(func(key, value gjson.Result) bool {
				user.Logins = append(user.Logins, value.Time())
				return true
			})
		} else {
			err = UnexpectedResponse
		}
	}
	return err
}

func (user *User) Save() error {
	_, err := DB.Mutate(user)
	return err
}

func (user *User) Validate(verifier string, identifier string) (string, error) {
	var token string
	err := user.Sync()
	if err != nil {
		return token, err
	}

	isValid := user.Verifier == verifier && time.Now().Before(user.Vexpiry)
	if !isValid {
		return token, ValidationCodeError
	}
	token, err = user.GenerateToken(identifier)
	if err != nil {
		return token, err
	}

	update := obj{
		"tokens": token,
		"logins": time.Now(),
	}

	if !user.Verified && isValid {
		update["verified"] = isValid
	}

	err = user.Update(update)
	if err == nil {
		_, err = DB.Delete(obj{
			"uid":      user.UID,
			"verifier": nil,
			"vexpiry":  nil,
		})
	}
	return token, err
}

func (user *User) Verify(firsttime bool) error {
	if !validUsername(user.Name) {
		return InvalidDetails
	}

	user.Verifier = randStr(VcodeSize)
	// Expire in 15 minutes & 30 seconds (30s for time it takes to send email, lag & so on)
	user.Vexpiry = time.Now().Add(time.Second * ((60 * 15) + 30))

	err := user.Update(obj{
		"verifier": user.Verifier,
		"vexpiry":  user.Vexpiry,
		"verified": false,
	})
	if err != nil {
		return err
	}

	verificationEmail := Email{
		To: []string{user.Email},
	}

	if firsttime {
		verificationEmail.Subject = Welcome.Verified
	} else {
		verificationEmail.Subject = Welcome.Unverified
	}

	var emailHTMLBuffer bytes.Buffer
	err = VerificationEmail.Execute(&emailHTMLBuffer, obj{
		"AppName":  AppName,
		"Username": user.Name,
		"Domain":   Domain,
		"VCode":    user.Verifier,
	})
	if err != nil {
		return err
	}

	verificationEmail.HTML = emailHTMLBuffer.Bytes()

	go SendEmail(verificationEmail)
	return nil
}

func createUser(name, email string) (User, error) {
	user := User{}
	if !validUsernameAndEmail(name, email) {
		return user, InvalidDetails
	}
	user.Name = name
	user.Email = email
	if !isUserUnique(&user) {
		return user, UserNotUnique
	}
	user.Created = time.Now()
	user.Description = "a New user who goes by " + user.Name
	user.Verified = false
	user.Roles[0] = Roles.User
	// user.Created.Format(time.RFC3339)
	return user, nil
}

func registerUser(name, email string) (User, error) {
	user, err := createUser(name, email)
	if err == nil {
		_, err = DB.Mutate(&user)
		user.Sync()
	}
	return user, err
}

func isUserUnique(user *User) bool {
	return DB.IndexWithValueExists("email", user.Email, true) && DB.IndexWithValueExists("username", user.Name, true)
}

func populateUserDetails(user *User, res DBresponse) error {
	users := gjson.GetBytes(res.GetJson(), "users").Array()
	if len(users) == 1 {
		userJSON := users[0]
		user.UID = userJSON.Get("uid").String()
		user.Email = userJSON.Get("email").String()
		user.Name = userJSON.Get("username").String()
		user.Created = userJSON.Get("created").Time()
		user.Verified = userJSON.Get("verified").Bool()
		user.Description = userJSON.Get("description").String()

		user.Tokens = []string{}

		userJSON.Get("tokens").ForEach(func(key, value gjson.Result) bool {
			user.Tokens = append(user.Tokens, value.String())
			return true
		})

		user.Logins = []time.Time{}
		userJSON.Get("logins").ForEach(func(key, value gjson.Result) bool {
			user.Logins = append(user.Logins, value.Time())
			return true
		})
		return nil
	}
	return UnexpectedResponse
}

func UserByUID(uid string) (User, error) {
	res, err := DB.QueryNoVars(`{
    users(func: uid("` + uid + `")) {
      uid
      email
      username
      description
      created
      logins
      tokens
      verified
    }
  }`)
	user := User{}
	if err == nil {
		err = populateUserDetails(&user, res)
	}
	return user, err
}

func UserByProp(prop, value string) (User, error) {
	res, err := DB.QueryNoVars(`{
    users(func: eq(` + prop + `, "` + value + `")) {
      uid
      email
      username
      description
      created
      logins
      tokens
      verified
    }
  }`)
	user := User{}
	if err == nil {
		err = populateUserDetails(&user, res)
	}
	return user, err
}

func UserByEmail(email string) (User, error) {
	return UserByProp("email", email)
}

func UserByName(name string) (User, error) {
	return UserByProp("username", name)
}

var (
	UnauthorizedError   = sendError("unauthorized request, cannot proceed")
	InvalidDetailsError = sendError("invalid details, could not authorize user")
	NotUniqueError      = sendError("user details must be valid and unique")
	BadNameError        = sendError("invalid username, could not authorize user")
	BadEmailError       = sendError("invalid email, could not authorize user")
	BadRequestError     = sendError("bad request, check details and try again")
	ServerJSONError     = sendError("ran into trouble decoding your request")
	ServerDBError       = sendError("server error, could not complete your request")
)

/*
	Actual Logic Starts Here
*/

func startUserService() {
	Welcome.Verified = "Login to " + AppName
	Welcome.Unverified = "Welcome to " + AppName

	Server.POST("/auth", func(c ctx) error {
		body, err := JSONbody(c)
		if err != nil {
			return BadRequestError(c)
		}

		email := body.Get("email").String()
		if !validEmail(email) {
			return BadEmailError(c)
		}

		username := body.Get("username").String()
		if !validUsername(username) {
			return BadNameError(c)
		}

		user, err := UserByEmail(email)
		if user.Name != username {
			return NotUniqueError(c)
		}

		if err == nil {
			user.Verify(false)
		} else if err == UserDoesNotExist {
			if DevMode {
				fmt.Println("user doesn't exist: ", username, email)
			}
			_, err = registerUser(username, email)
			if err == nil {
				user.Verify(true)
			}
		}

		if err == nil {
			return c.JSON(203, obj{
				"msg":   "Thank You " + user.Name + ", please check your email a verification email should arrive shortly.",
				"valid": true,
			})
		} else if DevMode {
			fmt.Println("Authentication Problem: \n\tusername - ", username, "\n\temail - ", email, "\n\terror - ", err)
		}

		return UnauthorizedError(c)
	})

	Server.GET("/auth/verify/:tk", func(c ctx) error {
		token := c.Param("tk")
		if len(token) <= 50 {
			return UnauthorizedError(c)
		}

		user, err := ValidateToken(token)
		if user == nil || err != nil {
			return UnauthorizedError(c)
		}

		return c.JSON(200, obj{"valid": true, "username": user.Name})
	})

	Server.GET("/auth/logout/:tk", func(c ctx) error {
		token := c.Param("tk")
		if len(token) <= 50 {
			return UnauthorizedError(c)
		}

		if DestroyToken(token) != nil {
			return UnauthorizedError(c)
		}

		return c.JSON(200, obj{"msg": "Sucessfully logged out", "valid": true})
	})

	Server.GET("/auth/:usr/:mode/:vcode", func(c ctx) error {
		username := c.Param("usr")
		vcode := c.Param("vcode")

		if !validUsername(username) {
			return BadNameError(c)
		}

		if len(vcode) != VcodeSize {
			return BadRequestError(c)
		}

		user, err := UserByName(username)
		if err != nil {
			return UnauthorizedError(c)
		}

		identifier := c.Request().Header.Get("User-Agent")

		token, err := user.Validate(vcode, identifier)
		if err != nil {
			panic(err)
		}
		if err == nil {
			if c.Param("mode") == "web" {
				return c.HTML(
					203,
					`<!DOCTYPE html>
					<html>
					<head>
						<meta charset="utf-8">
						<title>`+AppName+` Auth Redirect</title>
	    			<script>
							localStorage.setItem("token", "`+token+`")
							localStorage.setItem("username", "`+user.Name+`")
							location.replace("/")
						</script>
	  			</head>
					</html>`,
				)
			}
			return c.JSON(203, obj{"token": token, "username": username})
		}

		return UnauthorizedError(c)
	})
}

func RoleLimitedPath(role int64, handle func(c ctx, user *User) error) func(c ctx) error {
	return func(c ctx) error {
		body, err := JSONbody(c)
		if err != nil {
			return BadRequestError(c)
		}

		token := body.Get("token").String()
		user, err := ValidateToken(token)
		if err == nil {
			for _, r := range user.Roles {
				if r == role {
					return handle(c, user)
				}
			}
		}
		return UnauthorizedError(c)
	}
}
