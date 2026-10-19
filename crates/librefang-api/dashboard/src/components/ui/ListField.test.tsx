import { useState } from "react";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { ListField } from "./ListField";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, opts?: { defaultValue?: string }) =>
      opts?.defaultValue ?? (key === "common.remove" ? "Remove" : key),
  }),
}));

function Harness({
  initial = [],
  disabled,
}: {
  initial?: string[];
  disabled?: boolean;
}) {
  const [value, setValue] = useState(initial);
  return (
    <ListField
      value={value}
      onChange={setValue}
      placeholder="Add item"
      ariaLabel="Items"
      disabled={disabled}
    />
  );
}

const getInput = () => screen.getByRole("textbox", { name: "Items" });

describe("ListField", () => {
  it("commits the typed value on Enter", async () => {
    const user = userEvent.setup();
    render(<Harness />);

    await user.type(getInput(), "alpha{Enter}");

    expect(screen.getByText("alpha")).toBeInTheDocument();
    expect(getInput()).toHaveValue("");
  });

  it("commits the typed value on comma", async () => {
    const user = userEvent.setup();
    render(<Harness />);

    await user.type(getInput(), "alpha,");

    expect(screen.getByText("alpha")).toBeInTheDocument();
    expect(getInput()).toHaveValue("");
  });

  it("splits pasted comma, semicolon and newline separated text into chips", async () => {
    const user = userEvent.setup();
    render(<Harness />);

    const input = getInput();
    await user.click(input);
    await user.paste("alpha, beta\ngamma; delta");

    for (const item of ["alpha", "beta", "gamma", "delta"]) {
      expect(screen.getByText(item)).toBeInTheDocument();
    }
    expect(getInput()).toHaveValue("");
  });

  it("drops duplicates already in the list and within the paste", async () => {
    const user = userEvent.setup();
    render(<Harness initial={["alpha"]} />);

    await user.type(getInput(), "alpha{Enter}");
    expect(screen.getAllByText("alpha")).toHaveLength(1);

    const input = getInput();
    await user.click(input);
    await user.paste("beta, alpha, beta");
    expect(screen.getAllByText("alpha")).toHaveLength(1);
    expect(screen.getAllByText("beta")).toHaveLength(1);
  });

  it("removes the last chip with Backspace when the input is empty", async () => {
    const user = userEvent.setup();
    render(<Harness initial={["alpha", "beta"]} />);

    await user.click(getInput());
    await user.keyboard("{Backspace}");

    expect(screen.queryByText("beta")).not.toBeInTheDocument();
    expect(screen.getByText("alpha")).toBeInTheDocument();
  });

  it("commits the typed value when the field loses focus", async () => {
    const user = userEvent.setup();
    render(<Harness />);

    await user.type(getInput(), "gamma");
    await user.click(document.body);

    expect(screen.getByText("gamma")).toBeInTheDocument();
    expect(getInput()).toHaveValue("");
  });

  it("disables the input and hides the remove buttons when disabled", () => {
    render(<Harness initial={["alpha"]} disabled />);

    expect(getInput()).toBeDisabled();
    expect(screen.queryByRole("button", { name: "Remove alpha" })).not.toBeInTheDocument();
  });

  it("removes a chip when its remove button is clicked", async () => {
    const user = userEvent.setup();
    render(<Harness initial={["alpha", "beta"]} />);

    await user.click(screen.getByRole("button", { name: "Remove alpha" }));

    expect(screen.queryByText("alpha")).not.toBeInTheDocument();
    expect(screen.getByText("beta")).toBeInTheDocument();
  });
});
