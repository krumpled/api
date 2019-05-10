package server

// Options contains the configuration necessary for running the krumpled http.Server
type Options struct {
	Addr   string
	Google struct {
		ClientId     string
		ClientSecret string
		RedirectUri  string
	}
	Krumpled struct {
		RedirectUri string
	}
}
