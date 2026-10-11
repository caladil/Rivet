// **No dependencies, and that is not austerity.**
//
// Everything this service needs from a Rivet arrives as an environment
// variable or a file descriptor (`ADR-036` §3), and the standard library reads
// both. `rivet.go` is the whole of the platform binding.
module template-go-service

go 1.24
