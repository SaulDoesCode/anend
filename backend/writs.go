package backend

import (
	"context"
	"fmt"
	"time"

	"github.com/Machiel/slugify"
	"github.com/arangodb/go-driver"
)

// Writ - struct representing a post or document in the database
type Writ struct {
	Key         string      `json:"_key,omitempty" msgpack:"_key,omitempty"`
	Type        string      `json:"type,omitempty" msgpack:"type,omitempty"`
	Title       string      `json:"title,omitempty" msgpack:"title,omitempty"`
	AuthorKey   string      `json:"authorkey,omitempty" msgpack:"authorkey,omitempty"`
	Author      string      `json:"author,omitempty" msgpack:"author,omitempty"`
	Content     string      `json:"content,omitempty" msgpack:"content,omitempty"`
	Injection   string      `json:"injection,omitempty" msgpack:"injection,omitempty"`
	Markdown    string      `json:"markdown,omitempty" msgpack:"markdown,omitempty"`
	Description string      `json:"description,omitempty" msgpack:"description,omitempty"`
	Slug        string      `json:"slug,omitempty" msgpack:"slug,omitempty"`
	Tags        []string    `json:"tags,omitempty" msgpack:"tags,omitempty"`
	Edits       []time.Time `json:"edits,omitempty" msgpack:"edits,omitempty"`
	Created     time.Time   `json:"created,omitempty" msgpack:"created,omitempty"`
	Views       int64       `json:"views,omitempty" msgpack:"views,omitempty"`
	ViewedBy    []string    `json:"viewedby,omitempty" msgpack:"viewedby,omitempty"`
	LikedBy     []string    `json:"likedby,omitempty" msgpack:"likedby,omitempty"`
	Public      bool        `json:"public,omitempty" msgpack:"public,omitempty"`
	MembersOnly bool        `json:"membersonly,omitempty" msgpack:"membersonly,omitempty"`
	NoComments  bool        `json:"nocomments,omitempty" msgpack:"nocomments,omitempty"`
	Roles       []int64     `json:"roles,omitempty" msgpack:"roles,omitempty"`
}

// GetLink get a slug link with a key query param incase the title/slug changed
func (w *Writ) GetLink() string {
	return "https://" + AppDomain + "/writ/" + w.Slug + "?writ=" + w.Key
}

// Timeframe a distance of time between a .Start time and a .Finish time
type Timeframe struct {
	Start time.Time `json:"start,omitempty" msgpack:"start"`
	End   time.Time `json:"end,omitempty" msgpack:"end"`
}

// WritQuery a easier way to query the db for writs
type WritQuery struct {
	One                bool                   `json:"one,omitempty" msgpack:"one,omitempty"`
	Key                string                 `json:"_key,omitempty" msgpack:"_key,omitempty"`
	PrivateOnly        bool                   `json:"privateonly,omitempty" msgpack:"privateonly,omitempty"`
	IncludePrivate     bool                   `json:"includeprivate,omitempty" msgpack:"includeprivate,omitempty"`
	EditorMode         bool                   `json:"editormode,omitempty" msgpack:"editormode,omitempty"`
	Extensive          bool                   `json:"extensive,omitempty" msgpack:"extensive,omitempty"`
	UpdateViews        bool                   `json:"updateviews,omitempty" msgpack:"updateviews,omitempty"`
	Comments           bool                   `json:"comments,omitempty" msgpack:"comments,omitempty"`
	MembersOnly        bool                   `json:"membersonly,omitempty" msgpack:"membersonly,omitempty"`
	IncludeMembersOnly bool                   `json:"includemembersonly,omitempty" msgpack:"includemembersonly,omitempty"`
	DontSort           bool                   `json:"dontsort,omitempty" msgpack:"dontsort,omitempty"`
	Vars               map[string]interface{} `json:"vars,omitempty" msgpack:"vars,omitempty"`
	ViewedBy           string                 `json:"viewedby,omitempty" msgpack:"viewedby,omitempty"`
	LikedBy            string                 `json:"likedby,omitempty" msgpack:"likedby,omitempty"`
	Viewer             string                 `json:"viewer,omitempty" msgpack:"viewer,omitempty"`
	Title              string                 `json:"title,omitempty" msgpack:"title,omitempty"`
	Slug               string                 `json:"slug,omitempty" msgpack:"slug,omitempty"`
	Author             string                 `json:"author,omitempty" msgpack:"author,omitempty"`
	Created            time.Time              `json:"created,omitempty" msgpack:"created,omitempty"`
	Between            Timeframe              `json:"between,omitempty" msgpack:"between,omitempty"`
	Roles              []int64                `json:"roles,omitempty" msgpack:"roles,omitempty"`
	Limit              []int64                `json:"limit,omitempty" msgpack:"limit,omitempty"`
	Tags               []string               `json:"tags,omitempty" msgpack:"tags,omitempty"`
	Omissions          []string               `json:"omissions,omitempty" msgpack:"omissions,omitempty"`
}

// Exec execute a WritQuery to retrieve some/certain writs
func (q *WritQuery) Exec() ([]Writ, error) {
	writs := []Writ{}

	if q.One {
		writ, err := q.ExecOne()
		if err == nil {
			writs = append(writs, writ)
		}
		return writs, err
	}

	if q.Vars == nil {
		q.Vars = obj{}
	}

	query := "FOR writ IN writs "

	filter := ""
	firstfilter := true

	if q.PrivateOnly {
		if !firstfilter {
			filter += "&& "
		} else {
			firstfilter = false
		}
		filter += `writ.public == false `
	} else if !q.IncludePrivate {
		if !firstfilter {
			filter += "&& "
		} else {
			firstfilter = false
		}
		filter += `writ.public == true `
	}

	if q.MembersOnly {
		if !firstfilter {
			filter += "&& "
		}
		firstfilter = false
		filter += `writ.membersonly == true `
	} else if !q.IncludeMembersOnly {
		if !firstfilter {
			filter += "&& "
		}
		firstfilter = false
		filter += `writ.membersonly == false `
	}

	startzero := q.Between.Start.IsZero()
	endzero := q.Between.End.IsZero()
	if !startzero || !endzero {
		if !firstfilter {
			filter += "&& "
		}
		firstfilter = false
		if !startzero && !endzero {
			q.Vars["@betweenStart"] = q.Between.Start
			q.Vars["@betweenEnd"] = q.Between.End
			filter += "writ.created > @betweenStart && writ.created < @betweenEnd "
		} else if !startzero {
			q.Vars["@betweenStart"] = q.Between.Start
			filter += "writ.created > @betweenStart "
		} else if !endzero {
			q.Vars["@betweenEnd"] = q.Between.Start
			filter += "writ.created < @betweenEnd "
		}
	}

	if len(q.Author) > 0 {
		if !firstfilter {
			filter += "&& "
		}
		firstfilter = false
		q.Vars["author"] = q.Author
		filter += `writ.author == @author `
	}

	if len(q.ViewedBy) > 0 {
		if !firstfilter {
			filter += "&& "
		}
		firstfilter = false
		q.Vars["viewedby"] = q.ViewedBy
		filter += `@viewedby IN writ.viewedby `
	}

	if len(q.LikedBy) > 0 {
		if !firstfilter {
			filter += "&& "
		}
		firstfilter = false
		q.Vars["likedby"] = q.LikedBy
		filter += `@likedby IN writ.likedby `
	}

	if len(q.Roles) > 0 {
		if !firstfilter {
			filter += "&& "
		}
		firstfilter = false
		q.Vars["roles"] = q.Roles
		filter += `@roles ALL IN writ.roles `
	}

	if len(q.Tags) > 0 {
		if !firstfilter {
			filter += "&& "
		}
		firstfilter = false
		q.Vars["tags"] = q.Tags
		filter += `@tags ALL IN writ.tags `
	}

	if !firstfilter {
		query += "FILTER " + filter
	}

	if !q.DontSort {
		query += "SORT writ.created DESC "
	}

	if len(q.Limit) > 0 {
		q.Vars["pagenum"] = q.Limit[0]
		query += `LIMIT @pagenum`
		if len(q.Limit) == 2 {
			q.Vars["pagesize"] = q.Limit[1]
			query += `, @pagesize `
		}
	}

	query += " RETURN "

	final := "MERGE(writ, {likes: writ.likes + LENGTH(writ.likedby), views: writ.views + LENGTH(writ.viewedby)})"

	if !q.EditorMode {
		q.Omissions = append(q.Omissions, "markdown", "edits", "public", "roles", "authorkey")
	} else {
		q.Omissions = append(q.Omissions, "content")
	}

	if !q.Extensive {
		q.Omissions = append(q.Omissions, "likedby", "viewedby")
	}

	if len(q.Omissions) > 0 {
		q.Vars["omissions"] = q.Omissions
		final = "UNSET(" + final + ", @omissions)"
	}

	query += final

	if DevMode {
		fmt.Println("\n You're trying this query now: \n", query, "\n\t")
	}

	ctx := driver.WithQueryCount(context.Background())
	cursor, err := DB.Query(ctx, query, q.Vars)
	if err == nil {
		defer cursor.Close()
		for {
			var writ Writ
			_, err := cursor.ReadDocument(ctx, &writ)
			if driver.IsNoMoreDocuments(err) {
				break
			} else if err != nil {
				if DevMode {
					fmt.Println("DB Multiple Query - something strange happened: ", err)
				}
				panic(err)
			}
			writs = append(writs, writ)
		}
	} else if driver.IsNoMoreDocuments(err) {
		fmt.Println(`No more docs? Awww :( - `, err)
	} else if DevMode {
		fmt.Println("\n... And, it would seem that it has failed: \n", err, "\n\t")
	}
	return writs, err
}

// ExecOne execute a WritQuery to retrieve a single writ
func (q *WritQuery) ExecOne() (Writ, error) {
	var writ Writ

	if q.Vars == nil {
		q.Vars = obj{}
	}

	query := "FOR writ IN writs "

	filter := ""
	firstfilter := true

	if q.PrivateOnly {
		if !firstfilter {
			filter += "&& "
		} else {
			firstfilter = false
		}
		filter += `writ.public == false `
	} else if !q.IncludePrivate {
		if !firstfilter {
			filter += "&& "
		} else {
			firstfilter = false
		}
		filter += `writ.public == true `
	}

	if q.MembersOnly {
		if !firstfilter {
			filter += "&& "
		}
		firstfilter = false
		filter += `writ.membersonly == true `
	}

	if !q.Created.IsZero() {
		if !firstfilter {
			filter += "&& "
		}
		firstfilter = false
		q.Vars["@created"] = q.Created
		filter += "writ.created == @created "
	}

	if len(q.Key) > 0 {
		if !firstfilter {
			filter += "&& "
		}
		firstfilter = false
		q.Vars["key"] = q.Key
		filter += `writ._key == @key `
	}

	if len(q.Slug) > 0 {
		if !firstfilter {
			filter += "&& "
		}
		firstfilter = false
		q.Vars["slug"] = q.Slug
		filter += `writ.slug == @slug `
	}

	if len(q.Title) > 0 {
		if !firstfilter {
			filter += "&& "
		}
		firstfilter = false
		q.Vars["title"] = q.Title
		filter += `writ.title == @title `
	}

	if len(q.Author) > 0 {
		if !firstfilter {
			filter += "&& "
		}
		firstfilter = false
		q.Vars["author"] = q.Author
		filter += `writ.author == @author `
	}

	if len(q.ViewedBy) > 0 {
		if !firstfilter {
			filter += "&& "
		}
		firstfilter = false
		q.Vars["viewedby"] = q.ViewedBy
		filter += `@viewedby IN writ.viewedby `
	}

	if len(q.LikedBy) > 0 {
		if !firstfilter {
			filter += "&& "
		}
		firstfilter = false
		q.Vars["likedby"] = q.LikedBy
		filter += `@likedby IN writ.likedby `
	}

	if len(q.Roles) > 0 {
		if !firstfilter {
			filter += "&& "
		}
		firstfilter = false
		q.Vars["roles"] = q.Roles
		filter += `@roles ALL IN writ.roles `
	}

	if len(q.Tags) > 0 {
		if !firstfilter {
			filter += "&& "
		}
		firstfilter = false
		q.Vars["tags"] = q.Tags
		filter += `@tags ALL IN writ.tags `
	}

	if !firstfilter {
		query += "FILTER " + filter
	}

	if q.UpdateViews {
		query += "UPDATE writ WITH {"
		if len(q.Viewer) > 0 {
			q.Vars["viewer"] = q.Viewer
			query += "viewedby: PUSH(writ.viewedby, @viewer, true)"
		} else {
			query += "views: writ.views + 1"
		}
		query += "} IN writs "
	}

	query += "RETURN "

	final := "MERGE(writ, {likes: writ.likes + LENGTH(writ.likedby), views: writ.views + LENGTH(writ.viewedby)})"

	if !q.EditorMode {
		q.Omissions = append(q.Omissions, "markdown", "edits", "public", "roles", "authorkey")
	} else {
		q.Omissions = append(q.Omissions, "content")
	}

	if !q.Extensive {
		q.Omissions = append(q.Omissions, "likedby", "viewedby")
	}

	if len(q.Omissions) > 0 {
		q.Vars["omissions"] = q.Omissions
		final = "UNSET(" + final + ", @omissions)"
	}

	query += final

	if DevMode {
		fmt.Println("\n You're trying this query now: \n", query, "\n\t")
	}

	err := QueryOne(query, q.Vars, &writ)

	if DevMode && err != nil {
		fmt.Println("\n... And, it would seem that it has failed: \n", err, "\n\t")
	}

	return writ, err
}

// Slugify generate and set .Slug from .Title
func (w *Writ) Slugify() {
	w.Slug = slugify.Slugify(w.Title)
}

// RenderContent from .Markdown generate html and set .Content
func (w *Writ) RenderContent() {
	w.Content = string(renderMarkdown([]byte(w.Markdown), false)[:])
}

// ToObj convert writ into map[string]interface{}
func (w *Writ) ToObj(omissions ...string) obj {
	output := obj{}

	if len(w.Key) != 0 {
		output["_key"] = w.Key
	}
	if len(w.Type) != 0 {
		output["type"] = w.Type
	}
	if len(w.Title) != 0 {
		output["title"] = w.Title
	}
	if len(w.AuthorKey) != 0 {
		output["authorkey"] = w.AuthorKey
	}
	if len(w.Author) != 0 {
		output["author"] = w.Author
	}
	if len(w.Content) != 0 {
		output["content"] = w.Content
	}
	if len(w.Injection) != 0 {
		output["injection"] = w.Injection
	}
	if len(w.Markdown) != 0 {
		output["markdown"] = w.Markdown
	}
	if len(w.Description) != 0 {
		output["description"] = w.Description
	}
	if len(w.Slug) != 0 {
		output["slug"] = w.Slug
	}
	if len(w.Tags) != 0 {
		output["tags"] = w.Tags
	}
	if len(w.Edits) != 0 {
		output["edits"] = w.Edits
	}
	if !w.Created.IsZero() {
		output["created"] = w.Created
	}
	if w.Views != 0 {
		output["views"] = w.Views
	}
	if len(w.ViewedBy) != 0 {
		output["viewedby"] = w.ViewedBy
	}
	if len(w.LikedBy) != 0 {
		output["likedby"] = w.LikedBy
	}
	if &w.Public != nil {
		output["public"] = w.Public
	}
	if &w.MembersOnly != nil {
		output["membersonly"] = w.MembersOnly
	}
	if &w.NoComments != nil {
		output["nocomments"] = w.NoComments
	}
	if len(w.Roles) != 0 {
		output["roles"] = w.Roles
	}

	if len(omissions) != 0 {
		for _, omission := range omissions {
			delete(output, omission)
		}
	}

	return output
}

// Update update a writ's details using a map[string]interface{}
func (w *Writ) Update(query string, vars obj) error {
	if len(w.Key) < 0 {
		return ErrIncompleteWrit
	}
	vars["key"] = w.Key
	query = "FOR w in writs FILTER w._key == @key UPDATE w WITH " + query + " IN writs OPTIONS {keepNull: false, waitForSync: true} RETURN NEW"
	ctx := driver.WithQueryCount(context.Background())
	cursor, err := DB.Query(ctx, query, vars)
	defer cursor.Close()
	if err == nil {
		_, err = cursor.ReadDocument(ctx, w)
	}
	return err
}

// WritByKey retrieve user using their db document key
func WritByKey(key string) (Writ, error) {
	var writ Writ
	_, err := Writs.ReadDocument(context.Background(), key, &writ)
	return writ, err
}

// InitWrit initialize a new writ
func InitWrit(w *Writ) error {
	if len(w.Tags) < 1 {
		return ErrMissingTags
	}

	ctx := driver.WithWaitForSync(context.Background(), true)

	exists := true
	var err error
	var currentWrit Writ
	if len(w.Key) == 0 {
		if DevMode {
			fmt.Println("Searching For: ", w.Title)
		}
		currentWrit, err = (&WritQuery{
			EditorMode: true,
			Title:      w.Title,
		}).ExecOne()
		exists = err == nil
		err = nil
	}

	if !exists {
		w.Created = time.Now()
		if len(w.Markdown) < 1 || len(w.Title) < 1 || len(w.Author) < 1 {
			if DevMode {
				fmt.Println("InitWrit - it's horribly incomplete, fix it, add in author, title, and markdown")
			}
			return ErrIncompleteWrit
		}

		user, err := UserByUsername(w.Author)
		if err != nil {
			if DevMode {
				fmt.Println("InitWrit - author ("+w.Author+") is invalid or MIA: ", err)
			}
			return ErrAuthorIsNoUser
		}
		w.AuthorKey = user.Key

		w.RenderContent()
		if len(w.Slug) < 1 {
			w.Slugify()
		}

		meta, err := Writs.CreateDocument(ctx, w)
		if err != nil {
			if DevMode {
				fmt.Println(`InitWrit - creating a writ in the db: `, err)
			}
			return err
		}
		w.Key = meta.Key
	} else {
		if len(w.Key) == 0 {
			w.Key = currentWrit.Key
		}
		if len(w.Title) != 0 && currentWrit.Title == w.Title {
			w.Title = ""
			w.Slug = ""
		} else {
			w.Slugify()
		}
		if len(w.Content) != 0 && currentWrit.Content == w.Content {
			w.Content = ""
			w.Markdown = ""
		} else {
			w.RenderContent()
		}
		if len(w.Edits) < len(currentWrit.Edits) {
			w.Edits = append(w.Edits, currentWrit.Edits...)
		}
		w.Edits = append(w.Edits, time.Now())
		ctx = driver.WithMergeObjects(ctx, true)
		_, err := Writs.UpdateDocument(ctx, w.Key, w.ToObj("_key"))
		if err != nil {
			if DevMode {
				fmt.Println(`InitWrit - error updating a writ in the db: `, err)
			}
			return err
		}
		if !currentWrit.Public && w.Public {
			go notifySubscribers(w.Key)
		}
	}

	return nil
}

func notifySubscribers(writKey string) {
	writ, err := WritByKey(writKey)
	if err != nil {
		return
	}

	query := `FOR u IN users FILTER u.subscriber == true RETURN u`
	ctx := driver.WithQueryCount(context.Background())
	var users []User
	cursor, err := DB.Query(ctx, query, obj{})
	if err != nil {
		return
	}
	defer cursor.Close()
	users = []User{}
	for {
		var doc User
		_, err = cursor.ReadDocument(ctx, &doc)
		if driver.IsNoMoreDocuments(err) {
			break
		} else if err != nil {
			return
		}
		users = append(users, doc)
	}

	mail := MakeEmail()
	mail.Subject("Subscriber Update: Newly Published Writ")
	domain := AppDomain
	if DevMode {
		domain = "localhost:2443"
	}
	mail.HTML().Set(`
		<h4>There's a new writ: ` + writ.Title + `</h4>
		<p><a href="https://` + domain + "/writ/" + writ.Slug + `?writ=` + writ.Key + `">check it out</a></p>
		<sub><a href="https://` + domain + `/subscribe-toggle">unsubcribe</a></sub>
	`)
	for _, user := range users {
		mail.Bcc(user.Email)
	}
	go SendEmail(mail)
}

func initWrits() {
	Mak.GET("/writ/:slug", func(c ctx) error {
		slug := c.Param("slug").Value().String()

		wq := WritQuery{
			UpdateViews: true,
			Slug:        slug,
		}

		user, err := CredentialCheck(c)
		if err == nil {
			wq.Viewer = user.Key
			wq.IncludeMembersOnly = true
		} else {
			err = nil
		}

		writ, err := wq.ExecOne()

		if driver.IsNotFound(err) {
			wq.Slug = ""
			// incase the slug/title changed but the key stayed the same
			key := c.Param("writ").Value().String()
			if len(key) < 2 {
				return NoSuchWrit.Send(c)
			}

			wq.Key = key
			writ, err = wq.ExecOne()
			if err != nil {
				return NoSuchWrit.Send(c)
			}
		} else if err != nil {
			return SendErr(c, 503, err.Error())
		}

		writdata := writ.ToObj()

		writdata["Created"] = writ.Created.Format("1 Jan 2006")
		writdata["CreateDate"] = writ.Created

		editslen := len(writ.Edits)
		if editslen != 0 {
			writdata["ModifiedDate"] = writ.Edits[editslen-1]
		}

		writdata["URL"] = writ.GetLink()

		c.SetHeader("content-type", "text/html")
		err = PostTemplate.Execute(c, &writdata)
		if err != nil {
			if DevMode {
				fmt.Println("GET /writ/:slug - error executing the post template: ", err)
			}
		}
		return err
	})

	Mak.GET("/like-writ/:slug", AuthHandle(func(c ctx, user *User) error {
		slug := c.Param("slug").Value().String()
		if len(slug) < 1 {
			return BadRequestError.Send(c)
		}

		ctx := driver.WithKeepNull(driver.WithWaitForSync(driver.WithQueryCount(context.Background())), false)
		_, err := DB.Query(
			ctx,
			`FOR w IN writs FILTER w.slug == @slug UPDATE w WITH {
				 likedby: @user IN w.likedby ? REMOVE(w.likedby, @user) : PUSH(w.likedby, @user)
			 } IN writs`,
			obj{
				"slug": slug,
				"user": user.Key,
			},
		)
		if err != nil {
			return SendMsgpack(c, 500, obj{
				"err": "liking this writ failed somehow",
				"msg": err.Error(),
			})
		}
		return c.WriteMsgpack(obj{"msg": "success, writ liked!"})
	}))

	Mak.GET("/writs-by-tag/:tag/:page/:count", func(c ctx) error {
		tag := c.Param("tag").Value().String()
		if len(tag) < 1 {
			return BadRequestError.Send(c)
		}

		page, err := c.Param("page").Value().Int64()
		if err != nil {
			return BadRequestError.Send(c)
		}
		count, err := c.Param("count").Value().Int64()
		if err != nil {
			return BadRequestError.Send(c)
		}

		if count > 200 {
			return RequestQueryOverLimitMembers.Send(c)
		}

		q := &WritQuery{
			Limit: []int64{page, count},
			Tags:  []string{tag},
		}

		user, err := CredentialCheck(c)
		if err == nil && user != nil {
			if count > 200 {
				return RequestQueryOverLimitMembers.Send(c)
			}
			q.IncludeMembersOnly = true
		} else if count > 50 {
			return RequestQueryOverLimit.Send(c)
		}

		writs, err := q.Exec()
		if err != nil {
			return ServerDBError.Send(c)
		}
		return c.WriteMsgpack(writs)
	})

	Mak.GET("/writlist/:page/:count", func(c ctx) error {
		page, err := c.Param("page").Value().Int64()
		if err != nil {
			return BadRequestError.Send(c)
		}
		count, err := c.Param("count").Value().Int64()
		if err != nil {
			return BadRequestError.Send(c)
		}

		if count > 200 {
			return RequestQueryOverLimitMembers.Send(c)
		}

		q := &WritQuery{
			Limit: []int64{page, count},
		}

		user, err := CredentialCheck(c)
		if err == nil && user != nil {
			if count > 200 {
				return RequestQueryOverLimitMembers.Send(c)
			}
			q.IncludeMembersOnly = true
		} else if count > 50 {
			return RequestQueryOverLimit.Send(c)
		}

		writs, err := q.Exec()
		if err != nil {
			return ServerDBError.Send(c)
		}
		return c.WriteMsgpack(writs)
	})

	Mak.POST("/writ", AdminHandle(func(c ctx, user *User) error {
		var writ Writ
		err := c.Bind(&writ)
		if err != nil {
			return BadRequestError.Send(c)
		}

		if len(writ.Author) == 0 {
			writ.Author = user.Username
			writ.AuthorKey = user.Key
		} else if len(writ.AuthorKey) == 0 {
			writ.AuthorKey = user.Key
		}

		err = InitWrit(&writ)
		if err != nil {
			if !driver.IsNoMoreDocuments(err) {
				return SendMsgpack(c, 503, obj{"ok": false, "error": err})
			}
		}

		fmt.Println(`Baking Writs! - `, writ.Title)
		return SuccessMsg.Send(c)
	}))

	Mak.POST("/writ-query", AdminHandle(func(c ctx, user *User) error {
		var q WritQuery
		err := c.Bind(&q)
		if err != nil {
			return BadRequestError.Send(c)
		}

		var output interface{}
		if q.One {
			output, err = q.ExecOne()
		} else {
			output, err = q.Exec()
		}
		if err != nil {
			return ServerDecodeError.Send(c)
		}
		return c.WriteMsgpack(output)
	}))

	Mak.GET("/writ-delete/:key", AdminHandle(func(c ctx, user *User) error {
		key := c.Param("key").Value().String()
		if len(key) < 1 {
			return BadRequestError.Send(c)
		}

		ctx := driver.WithWaitForSync(context.Background(), true)
		_, err := Writs.RemoveDocument(ctx, key)
		if err != nil {
			return DeleteWritError.Send(c)
		}
		return c.WriteMsgpack(obj{"msg": "writ deleted, it's gone"})
	}))

	fmt.Println("Writ Service Started")
}
