namespace actual { int leaf(){return 1;} } namespace foreign { int leaf(); } int caller(){return foreign::leaf();}
