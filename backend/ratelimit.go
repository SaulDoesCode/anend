package backend

import (
	"context"
	"fmt"
	"time"

	"github.com/arangodb/go-driver"
)

type ratelimit struct {
	Key   string `json:"_key,omitempty"`
	Start int64  `json:"start"`
	Count int64  `json:"count"`
}

// ratelimitEmail limit how many emails can be sent at once
//
// nb. maxcount starts from 0
// to limit your user to 3 consecutive emails, set maxcount to 2
func ratelimitEmail(email string, maxcount int64, duration time.Duration) bool {
	var limit ratelimit
	err := QueryOne(
		`UPSERT {_key: @key}
		INSERT {_key: @key, start: @start, count: 0}
		UPDATE {count: OLD.count + 1} IN ratelimits OPTIONS {waitForSync: true}
		RETURN NEW`,
		obj{"start": time.Now().Unix(), "key": email},
		&limit,
	)
	if err != nil {
		if DevMode {
			fmt.Println("email ratelimits error: something happened ", err)
		}
		return false
	}

	if DevMode {
		fmt.Println(email, " - this email's ratelimiting expires then: ", time.Unix(limit.Start, 0).Add(duration))
	}

	if time.Since(time.Unix(limit.Start, 0).Add(duration)) > 0 {
		_, err := RateLimits.RemoveDocument(driver.WithWaitForSync(context.Background()), email)
		if DevMode && err != nil {
			fmt.Println("email ratelimits error: trouble resetting ", err)
		}
		return err == nil
	} else if limit.Count > maxcount {
		_, err := DB.Query(
			driver.WithWaitForSync(context.Background()),
			`FOR l IN ratelimits FILTER l._key == @key
			 UPDATE l WITH {count: l.count + 1, start: @start} IN ratelimits`,
			obj{"start": time.Now().Add(5 * time.Minute).Unix(), "key": email},
		)
		if DevMode && err != nil {
			fmt.Println("email ratelimits error: trouble removing entry ", err)
		}
		return false
	}

	return true
}
