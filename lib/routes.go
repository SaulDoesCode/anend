package backend

import (
	"fmt"
	"net/http"
)

func startRoutes() {
	requestDetails := `
	<code>
		AppName: %s<br>
		Protocol: %s<br>
		Host: %s<br>
		Remote Address: %s<br>
		Method: %s<br>
		Path: %s<br>
	</code>`

	Server.GET("/http-info", func(c ctx) error {
		req := c.Request()
		return c.HTML(
			http.StatusOK,
			fmt.Sprintf(
				requestDetails,
				AppName,
				req.Proto,
				req.Host,
				req.RemoteAddr,
				req.Method,
				req.URL.Path,
			),
		)
	})

}
