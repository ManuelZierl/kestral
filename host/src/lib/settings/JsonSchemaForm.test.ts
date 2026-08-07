import { fireEvent, render, screen } from "@testing-library/svelte";
import { describe, expect, it, vi } from "vitest";

import JsonSchemaForm from "./JsonSchemaForm.svelte";

describe("JsonSchemaForm", () => {
  const schema = {
    type: "object",
    properties: {
      max_iterations: { type: "integer", minimum: 1, maximum: 50 },
      show_thinking: { type: "boolean", description: "Show thinking" },
      custom_instructions: {
        type: "string",
        title: "Custom assistant instructions",
        maxLength: 16_384,
        "x-kestral-input": "multiline",
      },
      label: { type: "string", title: "Label" },
    },
    required: ["max_iterations"],
  };

  it("uses typed controls and preserves edits until save", async () => {
    const onSubmit = vi.fn();
    render(JsonSchemaForm, {
      schema,
      initialValue: {
        max_iterations: 10,
        show_thinking: false,
        custom_instructions: "First line\nSecond line",
        label: "Single line",
      },
      onSubmit,
    });

    const iterations = screen.getByLabelText("max_iterations") as HTMLInputElement;
    const thinking = screen.getByRole("checkbox", { name: /^show_thinking/ }) as HTMLInputElement;
    const instructions = screen.getByLabelText("Custom assistant instructions") as HTMLTextAreaElement;
    const label = screen.getByLabelText("Label");
    expect(iterations.type).toBe("number");
    expect(iterations.min).toBe("1");
    expect(iterations.max).toBe("50");
    expect(instructions).toBeInstanceOf(HTMLTextAreaElement);
    expect(instructions.maxLength).toBe(16_384);
    expect(instructions.value).toBe("First line\nSecond line");
    expect(label).toBeInstanceOf(HTMLInputElement);

    await fireEvent.input(iterations, { target: { value: "12" } });
    await fireEvent.click(thinking);
    await fireEvent.input(instructions, { target: { value: "Updated line one\nUpdated line two" } });
    expect(iterations.value).toBe("12");
    expect(thinking.checked).toBe(true);

    await fireEvent.submit(screen.getByRole("button", { name: "Save" }).closest("form")!);
    expect(onSubmit).toHaveBeenCalledWith({
      max_iterations: 12,
      show_thinking: true,
      custom_instructions: "Updated line one\nUpdated line two",
      label: "Single line",
    });
    expect(screen.getByRole("status").textContent).toBe("Settings saved.");

    await fireEvent.input(iterations, { target: { value: "13" } });
    expect(screen.queryByRole("status")).toBeNull();
  });
});
