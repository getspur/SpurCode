import { configuredValue } from "@config-probe";
import data from "./data.json" with { type: "json" };
export const result = {alias:configuredValue,json:data.message};
console.log(JSON.stringify(result));
