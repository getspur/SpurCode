int target(){return 1;} int caller(int (*target)()){return target();}
