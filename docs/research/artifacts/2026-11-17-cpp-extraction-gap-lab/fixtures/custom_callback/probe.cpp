void callback(){} void invoke(void (*f)()){f();} void caller(){invoke(callback);}
