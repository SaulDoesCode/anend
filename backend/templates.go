package backend

import (
	"bytes"
	"fmt"
	"io"
	"path/filepath"
	"sync"
	"text/template"

	tr "github.com/SaulDoesCode/transplacer"
	"github.com/fsnotify/fsnotify"
)

// Template to implement echo.Renderer
type Template struct {
	NoWatch   bool
	Templates string
	Watcher   *fsnotify.Watcher

	templates *template.Template
	sync.Mutex
}

// Render a template
func (t *Template) Render(c ctx, code int, name string, data interface{}) error {
	res := c.Response()
	res.Header().Set("Content-Type", "text/html")
	res.WriteHeader(code)
	return t.templates.ExecuteTemplate(c.Response(), name, data)
}

// Update tries to parse the templates again, and updates the Renderer accordingly
func (t *Template) Update() error {
	tmp, err := template.ParseGlob(tr.PrepPath(t.Templates, "*.*"))
	if err != nil {
		if DevMode {
			fmt.Println("Failed to parse updated templates: ", err)
		}
		return err
	}
	t.Lock()
	t.templates = tmp
	t.Unlock()
	return nil
}

// Init initializes the *Template (reading/parsing/watching)
func (t *Template) Init() error {
	abs, err := filepath.Abs(t.Templates)
	if err != nil {
		return err
	}
	t.Templates = abs
	t.Update()

	if !t.NoWatch {
		t.Watcher, err = fsnotify.NewWatcher()
		if err != nil {
			return fmt.Errorf("Templates failed to build file watcher: %v", err)
		}
		err = t.Watcher.Add(t.Templates)
		if err != nil {
			return fmt.Errorf("Templates failed to build file watcher: %v", err)
		}

		go func() {
			for {
				select {
				case e := <-t.Watcher.Events:
					if DevMode {
						fmt.Printf(
							"\nTemplate watcher event:\n\tfile: %s \n\t event %s\n",
							e.Name,
							e.Op.String(),
						)
					}

					t.Update()
				case err := <-t.Watcher.Errors:
					fmt.Println("nTemplate file watcher error: ", err)
				}
			}
		}()
	}

	return err
}

// Exec is a surfaced method to call ExecuteTemplates on the underlying template(s)
func (t *Template) Exec(writer io.Writer, name string, vars interface{}) error {
	return t.templates.ExecuteTemplate(writer, name, vars)
}

// AsBytes executes a template and writes it's output to a []byte
func (t *Template) AsBytes(name string, vars interface{}) ([]byte, error) {
	var buf bytes.Buffer
	err := t.templates.ExecuteTemplate(&buf, name, vars)
	return buf.Bytes(), err
}

// Renderer is the central app renderer
var Renderer *Template

func startTemplating() {
	Renderer = &Template{
		Templates: Conf.Templates,
	}
	err := Renderer.Init()
	if err != nil {
		panic(fmt.Sprintf("failed to start templating, check that all the templates are valid: %v", err))
	}

	if DevMode {
		fmt.Println("templates: ", Renderer.templates.DefinedTemplates())
	}
	Server.Renderer = Renderer
}
