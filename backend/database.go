package backend

import (
	"context"
	"crypto/tls"
	"fmt"
	"os"
	"time"

	"github.com/SaulDoesCode/air"
	"github.com/arangodb/go-driver"
	"github.com/arangodb/go-driver/http"
)

var (
	// DB central arangodb database for querying
	DB driver.Database
	// Users arangodb collection containing user data
	Users driver.Collection
	// Writs arangodb writ collection containing writs
	Writs driver.Collection
	// RateLimits arangodb ratelimits collection
	RateLimits driver.Collection
	// DBHealthTicker to see if the DB is still ok
	DBHealthTicker *time.Ticker
	// DBAlive does the db still live?
	DBAlive     bool
	dbendpoints []string
)

func setupDB(endpoints []string, dbname, username, password string) error {
	fmt.Println(`Attempting ArangoDB connection...`)

	// Create an HTTP connection to the database
	conn, err := http.NewConnection(http.ConnectionConfig{
		Endpoints: endpoints,
		TLSConfig: &tls.Config{
			ServerName:         AppDomain,
			InsecureSkipVerify: true,
		},
	})

	if err != nil {
		fmt.Println("Failed to create HTTP connection: ", err)
		return ErrBadDBConnection
	}

	client, err := driver.NewClient(driver.ClientConfig{
		Connection:     conn,
		Authentication: driver.JWTAuthentication(username, password),
	})
	if err != nil {
		fmt.Println("Could not get proper arangodb client:")
		return err
	}

	dbendpoints = endpoints

	db, err := client.Database(nil, dbname)
	if err != nil {
		fmt.Println("Could not get database object:")
		return err
	}

	DB = db
	users, err := DB.Collection(nil, "users")
	if err != nil {
		fmt.Println("Could not get users collection from db:")
		return err
	}
	Users = users

	writs, err := DB.Collection(nil, "writs")
	if err != nil {
		fmt.Println("Could not get users collection from db:")
		return err
	}
	Writs = writs

	_, _, err = Users.EnsureHashIndex(
		nil,
		[]string{"username", "email", "emailmd5"},
		&driver.EnsureHashIndexOptions{Unique: true},
	)
	if err != nil {
		return err
	}

	_, _, err = Users.EnsureHashIndex(
		nil,
		[]string{"verifier"},
		&driver.EnsureHashIndexOptions{Unique: true, Sparse: true},
	)
	if err != nil {
		return err
	}

	_, _, err = Writs.EnsureHashIndex(
		nil,
		[]string{"title", "tags", "slug"},
		&driver.EnsureHashIndexOptions{Unique: true},
	)
	if err != nil {
		return err
	}

	ratelimits, err := DB.Collection(nil, "ratelimits")
	if err != nil {
		fmt.Println("Could not get ratelimiting collection from db:")
		return err
	}
	RateLimits = ratelimits

	return err
}

var diedEmails = 0

func dbdiedEmergencyEmail(msg string, die bool) {
	if diedEmails != 0 {
		return
	}
	diedEmails++
	fmt.Println("Server.. Going.. Down, hang ten we might be reborn! - \n\t", msg)
	mail := MakeEmail()
	mail.To(MaintainerEmails...)
	mail.Subject("the/a " + AppDomain + " database has died, you need to fix it asap!")
	mail.Plain().Set(`
	msg:
	` + msg + `

	The server is going down, everything is going down.
	The app might save it self and restart the db.
	But, you should still come and check, to fix it just incase.
	
	the ssh command is: 
		
		ssh -L 8530:localhost:8530 root@grimstack.io

	Hurry up!!
	`)
	SendEmail(mail)

	if die {
		fmt.Println("the Database is kaput!!!")
		os.Exit(1)
	}
}

func startDBHealthCheck() {
	DBHealthTicker = time.NewTicker(20 * time.Second)
	go func() {
		for range DBHealthTicker.C {
			for _, endpoint := range dbendpoints {
				start := time.Now()
				DBAlive = Ping(endpoint)

				if !DBAlive && !DevMode {
					if time.Since(start) < 8*time.Millisecond {
						go func() {
							DBHealthTicker.Stop()
							go air.Shutdown(time.Second * 5)

							err := exeC(`nohup bash -c "nohup arangod -c /etc/arangodb3/arangod.conf &" && (sleep 10 && cd ` + AppLocation + ` && sudo ./main) & `)
							if err != nil {
								fmt.Println("could not redeem self in the face of hardship!: ", err)
								dbdiedEmergencyEmail("unable to self recusitate! :(", true)
							}

							dbdiedEmergencyEmail("something really bad happened with the db", true)
						}()
					} else {
						dbdiedEmergencyEmail("it seems the DB is remote, only you can save us now!", true)
					}
				}
			}
		}
	}()
	fmt.Println("database health checker started")
}

// Query query the app's DB with AQL, bindvars, and map that to an output
func Query(query string, vars obj) ([]obj, error) {
	var objects []obj
	ctx := driver.WithQueryCount(context.Background())
	cursor, err := DB.Query(ctx, query, vars)
	if err == nil {
		defer cursor.Close()
		objects = []obj{}
		for {
			var doc obj
			_, err = cursor.ReadDocument(ctx, &doc)
			if driver.IsNoMoreDocuments(err) || err != nil {
				break
			}
			objects = append(objects, doc)
		}
	}
	return objects, err
}

// QueryOne query the app's DB with AQL, bindvars, and map that to an output
func QueryOne(query string, vars obj, result interface{}) error {
	ctx := driver.WithQueryCount(context.Background())
	cursor, err := DB.Query(ctx, query, vars)
	if err == nil {
		_, err = cursor.ReadDocument(ctx, result)
		cursor.Close()
	}
	return err
}
