namespace a { int leaf(){return 1;} } namespace b { int leaf(){return 2;} } int caller(){return a::leaf()+b::leaf();}
