package backend

import (
	"encoding/json"
	"errors"
	"fmt"
	"time"

	"github.com/Machiel/slugify"
)

var (
	TooShort         = errors.New("writ is too short")
	MissingAuthors   = errors.New("writ needs an authors")
	URLNotUnique     = errors.New("writ url must be unique")
	TitleNotUnique   = errors.New("writ title must be unique")
	ContentNotUnique = errors.New("writ content must be unique")
	WritTooBig       = errors.New("writ is too big, 100kb max")
	WritDoesNotExist = errors.New("writ does not exist in db")

	Writ404Err = sendError("Writ does not exist in db")

	ViewCounter = make(chan ViewInc)
)

type ViewInc struct {
	UID   string `json:"uid,omitempty"`
	Views int64  `json:"views,omitempty"`
}

type Draft struct {
	UID     string    `json:"uid,omitempty"`
	Created time.Time `json:"created,omitempty"`
	Content string    `json:"content,omitempty"`
}

type Writs struct {
	Writs []Writ
}

var WritQuery = `{
  writs(func: eq(type, "post")) {
    uid
    url
    title
    body
    raw
    published
    views
    tag
    authorss
    created
  }
}`

// Writ a document created by a user, living in the graph
type Writ struct {
	UID        string      `json:"uid,omitempty"`
	URL        string      `json:"url,omitempty"`
	Type       string      `json:"type,omitempty"`
	Title      string      `json:"title,omitempty"`
	Body       string      `json:"body,omitempty"`
	Raw        string      `json:"raw,omitempty"`
	Published  bool        `json:"published"`
	Views      int64       `json:"views"`
	Visibility []int64     `json:"visibility,omitempty"`
	Tag        []string    `json:"tags,omitempty"`
	Modified   []time.Time `json:"modified,omitempty"`
	Created    time.Time   `json:"created,omitempty"`
	Authors    []*User     `json:"authors,omitempty"`
	CommentOn  *Writ       `json:"commenton,omitempty"`
	Draft      *Draft      `json:"draft,omitempty"`
	Comments   []*Writ     `json:"comments,omitempty"`
}

func uniqueTitle(title string) bool {
	return DB.IndexWithValueExists("title", title, true)
}
func uniqueURL(url string) bool {
	return DB.IndexWithValueExists("url", url, true)
}

func createWrit(writ *Writ) error {
	if writ.Authors == nil {
		return MissingAuthors
	}
	if !uniqueTitle(writ.Title) {
		return TitleNotUnique
	}
	if len(writ.URL) == 0 {
		writ.URL = slugify.Slugify(writ.Title)
	}
	if !uniqueURL(writ.URL) {
		return URLNotUnique
	}
	body := MD2HTML([]byte(writ.Raw))
	if len(body) > 102400 {
		return WritTooBig
	}
	if len(body) < 4 {
		return TooShort
	}
	writ.Body = string(body)
	if len(writ.Type) < 3 {
		writ.Type = "post"
	}
	writ.Created = time.Now()
	writ.Views = 0
	writ.Published = false
	_, err := DB.Mutate(&writ)
	return err
}

func updateWrit(uid string, raw string) error {
	writ := writByUID(uid, false)
	if writ == nil {
		return WritDoesNotExist
	}
	writ.Draft = &Draft{
		UID:     writ.UID,
		Content: writ.Raw,
		Created: time.Now(),
	}
	_, err := DB.Mutate(obj{
		"uid":      uid,
		"raw":      raw,
		"draft":    writ.Draft,
		"modified": writ.Draft.Created,
	})
	return err
}

func writByUID(uid string, incViews bool) *Writ {
	res, err := DB.QueryNoVars(`{
    writs(func: uid("` + uid + `")) {
      uid
      url
      title
      body
      raw
      published
      views
      tag
      created
      authors {
        username
      }
    }
  }`)
	if err == nil {
		var writs Writs
		if json.Unmarshal(res.GetJson(), &writs) == nil {
			if len(writs.Writs) == 1 {
				writ := writs.Writs[0]
				if incViews {
					ViewCounter <- ViewInc{
						UID:   writ.UID,
						Views: writ.Views + 1,
					}
				}
				return &writ
			}
		}
	}
	return nil
}

func writByTitle(title string, incViews bool) *Writ {
	res, err := DB.QueryNoVars(`{
    writs(func: eq(title, "` + title + `")) {
      uid
      url
      title
      body
      raw
      published
      views
      tag
      created
      authors {
        username
      }
    }
  }`)
	if err == nil {
		var writs Writs
		if json.Unmarshal(res.GetJson(), &writs) == nil {
			if len(writs.Writs) == 1 {
				writ := writs.Writs[0]
				if incViews {
					ViewCounter <- ViewInc{
						UID:   writ.UID,
						Views: writ.Views + 1,
					}
				}
				return &writ
			}
		}
	}
	return nil
}

func writByURL(url string, incViews bool) *Writ {
	res, err := DB.QueryNoVars(`{
    writs(func: eq(url, "` + url + `")) {
      uid
      url
      title
      body
      raw
      published
      views
      tag
      created
      authors {
        username
      }
    }
  }`)
	if err == nil {
		fmt.Println(string(res.GetJson()))
		var writs Writs
		if json.Unmarshal(res.GetJson(), &writs) == nil {
			if len(writs.Writs) == 1 {
				writ := writs.Writs[0]
				if incViews {
					ViewCounter <- ViewInc{
						UID:   writ.UID,
						Views: writ.Views + 1,
					}
				}
				return &writ
			}
		}
	}
	return nil
}

func writByTag(tag string, incViews bool) *Writ {
	res, err := DB.QueryNoVars(`{
    writs(func: allofterms(tag, "` + tag + `")) {
      uid
      url
      title
      body
      raw
      published
      views
      tag
      created
      authors {
        username
      }
    }
  }`)
	if err == nil {
		var writs Writs
		if json.Unmarshal(res.GetJson(), &writs) == nil {
			if len(writs.Writs) == 1 {
				writ := writs.Writs[0]
				if incViews {
					ViewCounter <- ViewInc{
						UID:   writ.UID,
						Views: writ.Views + 1,
					}
				}
				return &writ
			}
		}
	}
	return nil
}

func startWritService() {
	go func() {
		for {
			select {
			case vc := <-ViewCounter:
				DB.Mutate(&vc)
			}
		}
	}()

	Server.POST("/writ", func(c ctx) error {
		body, err := JSONbody(c)
		if err != nil {
			return BadRequestError(c)
		}
		err = WritDoesNotExist
		query := body.Map()
		if url, ok := query["url"]; ok {
			writ := writByURL(url.String(), true)
			if writ != nil {
				return c.JSON(200, writ)
			}
		} else if tag, ok := query["tag"]; ok {
			writ := writByTag(tag.String(), true)
			if writ != nil {
				return c.JSON(200, writ)
			}
		} else if update, ok := query["update"]; ok {
			uid := update.Get("uid").String()
			raw := update.Get("raw").String()
			if len(uid) > 2 && len(raw) > 4 {
				err = updateWrit(uid, raw)
			}
		} else if raw, ok := query["writ"]; ok {
			var writ Writ
			err = json.Unmarshal([]byte(raw.String()), &writ)
			if err != nil {
				return BadRequestError(c)
			}
			err = createWrit(&writ)
		}
		//token := c.Param("tk")
		//if len(token) == 0 {}
		if err == WritDoesNotExist {
			return Writ404Err(c)
		}
		if err != nil {
			return c.JSON(500, obj{"err": err.Error()})
		}
		return c.JSONBlob(200, []byte(`{"err":null}`))
	})
}
