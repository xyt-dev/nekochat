// Keep management and chat application graphs separate; only themes/layout are shared.
import "./themes";
import "./styles.css";

const root = document.getElementById("root")!;
if (location.pathname === "/admin" || location.pathname.startsWith("/admin/")) {
  void import("./admin/AdminApp").then(({ mountAdmin }) => mountAdmin(root));
} else {
  void import("./chat").then(({ mountChat }) => mountChat(root));
}
