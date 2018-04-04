package backend

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"

	"github.com/dgraph-io/dgo"
	"github.com/dgraph-io/dgo/protos/api"
	"github.com/tidwall/gjson"
	"google.golang.org/grpc"
)

type DBresponse = *api.Response
type DBoperation = *api.Operation

var (
	DBError = errors.New("Database error")
)

type Database struct {
	Client *dgo.Dgraph
	Conn   *grpc.ClientConn
}

func (db *Database) Mutate(Data interface{}) (*api.Assigned, error) {
	ctx := context.Background()
	mut := &api.Mutation{CommitNow: true}

	//op := &api.Operation{Schema: Schema}
	//err := db.Client.Alter(ctx, op)
	//if err != nil {return nil, err}

	plainJSON, err := json.Marshal(Data)
	if err != nil {
		return nil, err
	}

	mut.SetJson = plainJSON
	return db.Client.NewTxn().Mutate(ctx, mut)
}

func (db *Database) IndexWithValueExists(name, value string, unique bool) bool {
	ctx := context.Background()
	txn := db.Client.NewTxn()
	res, err := txn.Query(ctx, `{
		entities(func: eq(`+name+`, "`+value+`")) {
			uid
		}
	}`)
	if err == nil {
		entities := gjson.GetBytes(res.GetJson(), "entities").Array()
		if unique {
			return len(entities) < 2
		}
		return len(entities) > 0
	}
	return false
}

func (db *Database) IndexWithTextExists(name, value string, unique bool) bool {
	ctx := context.Background()
	txn := db.Client.NewTxn()
	res, err := txn.Query(ctx, `{
		entities(func: alloftext(`+name+`, "`+value+`")) {
			uid
		}
	}`)
	if err == nil {
		entities := gjson.GetBytes(res.GetJson(), "entities").Array()
		if unique {
			return len(entities) < 2
		}
		return len(entities) > 0
	}
	fmt.Println(err)
	return false
}

func (db *Database) Delete(data interface{}) (*api.Assigned, error) {
	ctx := context.Background()
	mut := &api.Mutation{CommitNow: true}

	//op := &api.Operation{Schema: Schema}
	//err := db.Client.Alter(ctx, op)
	//if err != nil {return nil, err}

	plainJSON, err := json.Marshal(data)
	if err != nil {
		return nil, err
	}

	mut.DeleteJson = plainJSON
	return db.Client.NewTxn().Mutate(ctx, mut)
}

func (db *Database) Query(query string, vars map[string]string) (*api.Response, error) {
	ctx := context.Background()
	txn := db.Client.NewTxn()
	return txn.QueryWithVars(ctx, query, vars)
}

func (db *Database) QueryNoVars(query string) (*api.Response, error) {
	ctx := context.Background()
	txn := db.Client.NewTxn()
	return txn.Query(ctx, query)
}

func initDB(address string) *Database {
	conn, err := grpc.Dial(address, grpc.WithInsecure())
	if err != nil {
		panic(err)
	}
	client := dgo.NewDgraphClient(api.NewDgraphClient(conn))
	// defer conn.Close()
	return &Database{
		Client: client,
		Conn:   conn,
	}
}
