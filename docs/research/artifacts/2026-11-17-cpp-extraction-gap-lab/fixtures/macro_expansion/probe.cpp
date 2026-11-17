#define RUN(f) f()
void callback(){} void caller(){RUN(callback);}
