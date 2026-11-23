function outer() {
  setup();
  return () => {
    if (ready) run();
  };
}
