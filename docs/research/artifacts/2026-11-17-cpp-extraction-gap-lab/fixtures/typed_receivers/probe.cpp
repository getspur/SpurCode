struct A { int run(){return 1;} }; struct B { int run(){return 2;} }; int caller(A &a,B &b){return a.run()+b.run();}
