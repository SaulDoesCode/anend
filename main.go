package main

import (
	"flag"

	"github.com/SaulDoesCode/Backend/lib"
)

func main() {
	confLocation := *flag.String("conf", "./private/conf.toml", "location of toml config file")

	backend.Init(confLocation)
}
