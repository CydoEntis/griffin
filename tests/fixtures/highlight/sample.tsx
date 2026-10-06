// A sample for the highlight tests.
import { Button } from "./button";

type Props = { title: string };

export function App({ title }: Props) {
  const go = () => alert(title);
  return <Button onClick={go}>{title}</Button>;
}
