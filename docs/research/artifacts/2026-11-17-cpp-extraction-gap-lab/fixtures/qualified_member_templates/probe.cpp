namespace n { template<class T> T identity(T x){return x;} } struct A { template<class T> T apply(T x){return x;} }; int caller(A &a){return n::identity<int>(1)+a.apply<int>(2);}
