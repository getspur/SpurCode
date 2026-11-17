#include "api.hpp"
struct A { int run(){return 1;} }; int caller(external::B &b){return b.run();}
