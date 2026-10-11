// **Start.** Last in the concatenation, because every view has to have
// registered itself into `VIEWS` before the first machine is selected.

boot().catch((e) => {
  $("#cards").textContent = "";
  $("#cards").append(el("div", "fail", e.message));
});
